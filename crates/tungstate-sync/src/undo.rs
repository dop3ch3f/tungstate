//! Taking a sync run back, on every member.
//!
//! A run is journalled as renames (every set-aside is one) and copies, so its
//! inverse is renames back and copies taken off again, in reverse order. The
//! baseline needs nothing: it is append-only, and marking the run undone
//! retires every reading it wrote, so the readings before it are current
//! again.
//!
//! Checked, not forced, and checked before anything moves: a copy is only
//! taken off if it still holds what was delivered, and a file only goes back
//! to a name nothing else has taken since. Finding out half way would leave
//! the members in a shape nobody asked for.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use tungstate_core::dupes::{Digest as _, free_name_by};
use tungstate_core::snapshot::{QUARANTINE, is_reserved};
use tungstate_execute::Reversal;
use tungstate_journal::{
    AppliedPlan, Journal, Location, NewOp, OpKind, Outcome, PlanId, Purpose, Reading, Sync,
};

use crate::{Opened, Place, Result, SyncError, explain, prune, slashed};

/// What taking a run back did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Undone {
    /// The run that was taken back.
    pub plan: Option<PlanId>,
    /// Files moved back to where the run found them.
    pub put_back: usize,
    /// Copies the run delivered, taken off again.
    pub taken_off: usize,
    /// Conflicting versions the run parked, left where they are.
    pub parked_left: usize,
    /// (member, path): deletions that caused the run, undone too, so the
    /// next run brings the file back to the member it was deleted on.
    pub revived: Vec<(String, String)>,
}

/// The last `last` runs of a sync not yet taken back, newest first.
///
/// # Errors
/// [`SyncError::Journal`] if the runs cannot be read.
pub fn standing(journal: &Journal, sync: &Sync, last: usize) -> Result<Vec<AppliedPlan>> {
    Ok(journal
        .sync_runs(sync, usize::MAX >> 1)?
        .into_iter()
        .filter(|plan| !plan.is_undone())
        .take(last)
        .collect())
}

/// One step, decided and checked, ready to carry out.
enum Step {
    /// A delivered copy: set aside where the member can rename, deleted
    /// where it cannot. Deleting is safe there: nothing can have been set
    /// aside on such a member, so the name held nothing before the copy.
    TakeOff {
        place: usize,
        path: String,
        to: Option<String>,
    },
    Move {
        place: usize,
        from: String,
        to: String,
    },
}

/// Take one run of a sync back on every member.
///
/// # Errors
/// [`SyncError::Refused`] naming why: not a run of this sync, already taken
/// back, a later run still standing, a run that deleted outright, or a
/// member that has moved on since.
#[allow(clippy::too_many_lines)]
pub fn undo(opened: &Opened, journal: &Journal, plan: PlanId) -> Result<Undone> {
    let sync = &opened.sync;
    let recorded = journal.plan_by_id(plan)?;
    let refuse = |why: String| Err(SyncError::Refused(why));
    if recorded.purpose != Some(Purpose::Sync)
        || recorded.folder != format!("sync:{}", sync.name)
        || recorded.applied_at < sync.created_at
        || recorded.is_an_undo()
    {
        return refuse(format!("plan {} is not a run of `{}`", plan.0, sync.name));
    }
    if recorded.is_undone() {
        return refuse(format!(
            "run {} of `{}` is already put back",
            plan.0, sync.name
        ));
    }
    // Runs come back in the order they went, newest first. A later run's
    // readings would still be current over this one's, and would read the
    // files put back as edits to carry everywhere.
    if let Some(later) = standing(journal, sync, usize::MAX)?
        .iter()
        .find(|run| run.id.0 > plan.0)
    {
        return refuse(format!(
            "run {} came after it; put that back first",
            later.id.0
        ));
    }
    let ops = journal.ops_for_plan(plan)?;
    let reversals = match tungstate_execute::invert_sync(&ops) {
        Ok(reversals) if recorded.reversible => reversals,
        Ok(_) | Err(_) => {
            return refuse(format!(
                "run {} deleted files outright (--on-remove delete); nothing can put them back",
                plan.0
            ));
        }
    };

    let place_of = |location: &Location| -> Result<usize> {
        opened
            .places
            .iter()
            .position(|p| p.holds(location))
            .ok_or_else(|| {
                SyncError::Refused(format!(
                    "`{}` is not a member of `{}` any more",
                    location.display_path(),
                    sync.name
                ))
            })
    };

    // The members as they would be part way through, so each step is checked
    // against what the steps before it will have done.
    let mut paper: BTreeMap<(usize, String), bool> = BTreeMap::new();
    let exists = |paper: &BTreeMap<(usize, String), bool>, place: usize, path: &str| {
        paper
            .get(&(place, path.to_string()))
            .copied()
            .unwrap_or_else(|| opened.places[place].backend.stat(Path::new(path)).is_ok())
    };
    let mut steps = Vec::new();
    let mut undone = Undone {
        plan: Some(plan),
        ..Undone::default()
    };
    for reversal in &reversals {
        match reversal {
            Reversal::Unsend { at, hash, size } => {
                let place = place_of(at)?;
                let path = slashed(&at.path);
                if is_reserved(&path) {
                    undone.parked_left += 1;
                    continue;
                }
                if !exists(&paper, place, &path) {
                    // Already gone: a half-finished undo being run again.
                    continue;
                }
                still_as_delivered(&opened.places[place], journal, &path, hash.as_ref(), *size)?;
                let to = opened.places[place]
                    .backend
                    .capabilities()
                    .atomic_rename
                    .then(|| {
                        free_name_by(&format!("{QUARANTINE}/{path}"), |candidate| {
                            exists(&paper, place, candidate)
                        })
                    });
                paper.insert((place, path.clone()), false);
                if let Some(to) = &to {
                    paper.insert((place, to.clone()), true);
                }
                steps.push(Step::TakeOff { place, path, to });
            }
            Reversal::Unrename { now_at, back_to } => {
                let place = place_of(now_at)?;
                let (from, to) = (slashed(&now_at.path), slashed(&back_to.path));
                let member = &opened.places[place].member.name;
                match (exists(&paper, place, &from), exists(&paper, place, &to)) {
                    (true, false) => {}
                    // Already back.
                    (false, true) => continue,
                    (true, true) => {
                        return refuse(format!(
                            "`{to}` on `{member}` has been taken since; nothing was put back"
                        ));
                    }
                    (false, false) => {
                        return refuse(format!(
                            "`{from}` on `{member}` is gone; nothing was put back"
                        ));
                    }
                }
                paper.insert((place, from.clone()), false);
                paper.insert((place, to.clone()), true);
                steps.push(Step::Move { place, from, to });
            }
        }
    }

    let id = journal.begin_plan_for(
        &format!("sync:{}", sync.name),
        &format!("undo of plan {}", plan.0),
        Some(plan),
        true,
        Some(Purpose::Sync),
    )?;
    for step in &steps {
        let (place, op, done) = match step {
            Step::TakeOff { place, path, to } => (
                *place,
                NewOp {
                    kind: if to.is_some() {
                        OpKind::Rename
                    } else {
                        OpKind::Remove
                    },
                    source: Some(opened.places[*place].at(path)),
                    destination: to.as_ref().map(|to| opened.places[*place].at(to)),
                    size: None,
                    link: None,
                    link_id: None,
                },
                &mut undone.taken_off,
            ),
            Step::Move { place, from, to } => (
                *place,
                NewOp {
                    kind: OpKind::Rename,
                    source: Some(opened.places[*place].at(from)),
                    destination: Some(opened.places[*place].at(to)),
                    size: None,
                    link: None,
                    link_id: None,
                },
                &mut undone.put_back,
            ),
        };
        let backend = opened.places[place].backend.as_ref();
        let entry = journal.begin(&op)?;
        journal.attach_to_plan(entry, id)?;
        let from = op
            .source
            .as_ref()
            .map(|l| slashed(&l.path))
            .unwrap_or_default();
        let result = match op.destination.as_ref().map(|l| slashed(&l.path)) {
            Some(to) => Path::new(&to)
                .parent()
                .map_or(Ok(()), |parent| backend.create_dir_all(parent))
                .and_then(|()| backend.rename(Path::new(&from), Path::new(&to))),
            None => backend.remove_file(Path::new(&from)),
        };
        match result {
            Ok(()) => {
                journal.finish(entry, &Outcome::Committed { hash: None })?;
                *done += 1;
                prune(backend, &from);
            }
            Err(error) => {
                let why = explain(&error);
                journal.finish(entry, &Outcome::Failed { error: why.clone() })?;
                // Stopped rather than carried on: the members are now part
                // way back, and running the undo again finishes the rest.
                return refuse(format!(
                    "stopped part way on `{}`: {why}; run it again to finish",
                    opened.places[place].member.name
                ));
            }
        }
    }
    journal.mark_undone(plan)?;
    undone.revived = revive(opened, journal, plan, id, &ops)?;
    Ok(undone)
}

/// A copy is only taken off while it is still what the run delivered.
fn still_as_delivered(
    place: &Place,
    journal: &Journal,
    path: &str,
    hash: Option<&String>,
    size: Option<u64>,
) -> Result<()> {
    let changed = || {
        SyncError::Refused(format!(
            "`{path}` on `{}` has changed since the run delivered it; nothing was put back",
            place.member.name
        ))
    };
    let meta = place.backend.stat(Path::new(path)).map_err(|_| changed())?;
    if size.is_some_and(|size| size != meta.len) {
        return Err(changed());
    }
    if let Some(hash) = hash {
        let mut digest = tungstate_execute::digest::Cached::new(
            place.backend.as_ref(),
            journal,
            &place.described,
        );
        let now = digest.whole(path).map_err(|why| SyncError::Read {
            member: place.member.name.clone(),
            path: path.to_string(),
            why,
        })?;
        if &now != hash {
            return Err(changed());
        }
    }
    Ok(())
}

/// The members a run recorded as gone without taking anything off them: the
/// ones a person deleted by hand, which the run then carried everywhere.
/// With the run taken back, their readings say "held" again while the file
/// is still missing there, and the next run would carry the deletion out all
/// over again. Remembering them as absent instead makes them ordinary empty
/// members, and the next run brings the file back to them.
fn revive(
    opened: &Opened,
    journal: &Journal,
    plan: PlanId,
    undo: PlanId,
    ops: &[tungstate_journal::Op],
) -> Result<Vec<(String, String)>> {
    let touched: BTreeSet<(i64, String)> = ops
        .iter()
        .flat_map(|op| [op.source.as_ref(), op.destination.as_ref()])
        .flatten()
        .filter_map(|location| {
            let place = opened.places.iter().find(|p| p.holds(location))?;
            Some((place.member.id.0, slashed(&location.path)))
        })
        .collect();
    let current: BTreeMap<(i64, String), bool> = journal
        .baseline_for(opened.sync.id)?
        .into_iter()
        .map(|r| ((r.member.0, r.path), r.present))
        .collect();
    let mut revived = Vec::new();
    let mut readings = Vec::new();
    for reading in journal.readings_for_plan(plan)? {
        let key = (reading.member.0, reading.path.clone());
        if reading.present || touched.contains(&key) || current.get(&key) != Some(&true) {
            continue;
        }
        let Some(place) = opened.places.iter().find(|p| p.member.id == reading.member) else {
            continue;
        };
        if place.backend.stat(Path::new(&reading.path)).is_ok() {
            continue;
        }
        revived.push((place.member.name.clone(), reading.path.clone()));
        readings.push(Reading {
            present: false,
            size: 0,
            mtime: None,
            hash: None,
            ..reading
        });
    }
    journal.record_baseline(&readings, undo)?;
    Ok(revived)
}
