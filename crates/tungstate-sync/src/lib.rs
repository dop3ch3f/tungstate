//! Running a sync.
//!
//! [`open`] reaches every member, [`decide`] surveys them and asks
//! `tungstate_core::sync` what should move (reading whatever bytes it asks
//! for), and [`run`] carries it out and remembers what happened. Three steps
//! because a preview is the first two and changes nothing. [`undo`] takes a
//! run back on every member.
//!
//! A run is three phases. Every copy that has to be taken off a member, set
//! aside or deleted, goes first, and every rename; each is journalled before
//! it happens, the way a reorganisation's are, so `undo` can reverse it. Then
//! the legs: an ordinary link the sync owns per pair of members, so every rail
//! the drain has earned comes with it: temp names, verify before commit,
//! resume after a crash, and the destination pinned so a vanished volume stops
//! the run. Last, the old names of carried tidies on members that cannot
//! rename, taken off only where the new name landed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tungstate_backend::{Backend, RootToken};
use tungstate_core::dupes::Digest as _;
use tungstate_core::snapshot::{QUARANTINE, is_reserved};
use tungstate_core::sync::{self as core, Digests, Removal, Removed, SAMPLED, SyncOp, SyncPlan};
use tungstate_journal::{
    ConflictAction, Endpoint, Journal, LinkId, Location, MemberId, NewLink, NewOp, OpKind, Order,
    Outcome, PlanId, Purpose, Reading, SourcePolicy, Sync, SyncDirection, ends,
};
use tungstate_secret::SecretStore;
use tungstate_transfer::{FixedResolver, Progress, Summary, Transfer};

mod undo;
pub use undo::{Undone, standing, undo};

#[cfg(test)]
mod tests;

/// Anything that stops a sync, as a sentence naming the member at fault.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Journal(#[from] tungstate_journal::JournalError),
    #[error("cannot reach `{member}`: {why}")]
    Unreachable { member: String, why: String },
    #[error("cannot read `{member}`: {why}")]
    Unreadable { member: String, why: String },
    #[error("cannot read {path} on `{member}`: {why}")]
    Read {
        member: String,
        path: String,
        why: String,
    },
    #[error("the leg from `{from}` to `{to}` stopped: {why}")]
    Leg {
        from: String,
        to: String,
        why: String,
    },
    #[error("`{member}` is not the volume this run started on; stopped with nothing more moved")]
    Vanished { member: String },
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    NotYet(String),
}

type Result<T> = std::result::Result<T, SyncError>;

/// One member, reached.
pub struct Place {
    pub member: tungstate_journal::Member,
    pub end: Endpoint,
    pub backend: Box<dyn Backend>,
    /// How the member is written for a person: `~/CapCut` or `nas:capcut`.
    pub described: String,
    pub networked: bool,
}

impl Place {
    /// Where a path on this member is, as the journal records it.
    fn at(&self, path: &str) -> Location {
        Location::within(&self.end, PathBuf::from(path))
    }

    /// Whether a location the journal recorded is on this member.
    fn holds(&self, location: &Location) -> bool {
        location.connection == self.end.connection && location.root == self.end.path
    }
}

/// A sync with every member reached.
pub struct Opened {
    pub sync: Sync,
    pub places: Vec<Place>,
}

impl Opened {
    fn place(&self, id: i64) -> &Place {
        self.places
            .iter()
            .find(|p| p.member.id.0 == id)
            .expect("a plan only names members it was given")
    }

    /// A member's name, by the id a plan uses.
    #[must_use]
    pub fn name(&self, id: i64) -> &str {
        &self.place(id).member.name
    }
}

/// Reach every member of a sync.
///
/// # Errors
/// [`SyncError::Unreachable`] naming the member that could not be reached.
pub fn open(journal: &Journal, name: &str, secrets: &dyn SecretStore) -> Result<Opened> {
    let sync = journal.sync_by_name(name)?;
    let mut places = Vec::with_capacity(sync.members.len());
    for member in &sync.members {
        let end = match member.connection {
            Some(connection) => Endpoint::remote(connection, PathBuf::from(&member.path)),
            None => Endpoint::local(&member.path),
        };
        let backend = tungstate_backend_opendal::open(&end, journal, secrets).map_err(|e| {
            SyncError::Unreachable {
                member: member.name.clone(),
                why: e.to_string(),
            }
        })?;
        let networked = backend.capabilities().networked;
        places.push(Place {
            described: ends::describe(&end, journal),
            member: member.clone(),
            end,
            backend,
            networked,
        });
    }
    Ok(Opened { sync, places })
}

/// A run, decided, with what it cost to decide.
#[derive(Debug, Clone)]
pub struct Decided {
    pub plan: SyncPlan,
    pub members: Vec<core::Member>,
    pub digests: Digests,
    /// Files read to decide, and how many of those reads were samples.
    pub read: usize,
    pub sampled: usize,
    /// Of `read`, how many because the member keeps no modification times.
    pub read_for_no_times: usize,
    /// Of `read`, how many to tell a moved file from a new one.
    pub read_for_moves: usize,
    /// Of `read`, how many to recognise a conflict already parked.
    pub read_for_parked: usize,
    /// Per member, how many files the baseline says it held after the last
    /// run, so an empty listing can be told from an empty folder.
    pub held: BTreeMap<i64, usize>,
}

/// Survey every member and decide, reading what the decision asks to read.
///
/// # Errors
/// As [`decide_with`].
pub fn decide(opened: &Opened, journal: &Journal) -> Result<Decided> {
    decide_with(opened, journal, &core::Asked::default())
}

/// [`decide`], with conflicts settled or paths forgotten by hand.
///
/// # Errors
/// [`SyncError::Unreadable`] if a member cannot be listed, or
/// [`SyncError::Read`] if a file the decision needs cannot be read.
///
/// # Panics
/// Never in practice: the probe policy is a constant that has its own test.
#[allow(clippy::too_many_lines)]
pub fn decide_with(opened: &Opened, journal: &Journal, asked: &core::Asked) -> Result<Decided> {
    let probe = tungstate_core::policy::Policy::parse(tungstate_core::learn::PROBE)
        .expect("the probe policy parses")
        .policy;
    let mut members = Vec::with_capacity(opened.places.len());
    for place in &opened.places {
        let unreadable = |e: &dyn std::error::Error| SyncError::Unreadable {
            member: place.member.name.clone(),
            why: explain(e),
        };
        let snapshot =
            tungstate_attrs::survey(place.backend.as_ref(), &probe).map_err(|e| unreadable(&e))?;
        members.push(core::Member {
            id: place.member.id.0,
            name: place.member.name.clone(),
            local: place.member.connection.is_none() && !place.networked,
            connection: place.member.connection.map(|c| c.0),
            case_sensitive: snapshot.case_sensitive,
            renames: place.backend.capabilities().atomic_rename,
            files: core::Member::listing(&snapshot),
            parked: parked(place.backend.as_ref()).map_err(|e| unreadable(&e))?,
        });
    }

    let mut held: BTreeMap<i64, usize> = BTreeMap::new();
    let baseline: core::Baseline = journal
        .baseline_for(opened.sync.id)?
        .into_iter()
        .map(|r| {
            if r.present {
                *held.entry(r.member.0).or_default() += 1;
            }
            (
                (r.member.0, r.path),
                core::Seen {
                    present: r.present,
                    size: r.size,
                    mtime: r.mtime,
                    hash: r.hash,
                },
            )
        })
        .collect();
    let mode = mode_of(&opened.sync, now_ms());

    let mut decided = Decided {
        plan: SyncPlan::default(),
        members,
        digests: Digests::new(),
        read: 0,
        sampled: 0,
        read_for_no_times: 0,
        read_for_moves: 0,
        read_for_parked: 0,
        held,
    };
    // A few rounds settle any path: one to learn a file moved, one to see
    // whether two copies agree, one to see whether a conflict is already
    // parked. The bound is there so a bug cannot become a hang.
    for _ in 0..8 {
        let plan = core::decide_with(&decided.members, &baseline, &decided.digests, &mode, asked);
        if plan.questions.is_empty() {
            decided.plan = plan;
            return Ok(decided);
        }
        for question in &plan.questions {
            let place = opened.place(question.member);
            let mut digest = tungstate_execute::digest::Cached::new(
                place.backend.as_ref(),
                journal,
                &place.described,
            );
            let answer = if question.sampled {
                digest
                    .partial(&question.path)
                    .map(|d| format!("{SAMPLED}{d}"))
            } else {
                digest.whole(&question.path)
            }
            .map_err(|why| SyncError::Read {
                member: place.member.name.clone(),
                path: question.path.clone(),
                why,
            })?;
            decided.read += 1;
            decided.sampled += usize::from(question.sampled);
            match question.because {
                core::Because::NoTimes => decided.read_for_no_times += 1,
                core::Because::Moved => decided.read_for_moves += 1,
                core::Because::Parked => decided.read_for_parked += 1,
                core::Because::Agreement => {}
            }
            decided
                .digests
                .insert((question.member, question.path.clone()), answer);
        }
    }
    Err(SyncError::NotYet(
        "deciding kept asking for more reads; this is a bug, and nothing was moved".into(),
    ))
}

/// What a member's set-aside area holds, by root-relative path.
///
/// Listed rather than stat'ed, so "nothing set aside" and "cannot be reached"
/// stay different answers.
fn parked(backend: &dyn Backend) -> tungstate_backend::Result<BTreeMap<String, core::Now>> {
    let root = Path::new(QUARANTINE);
    let mut found = BTreeMap::new();
    if !backend
        .read_dir(Path::new(""))?
        .iter()
        .any(|entry| entry.meta.is_dir && entry.path == root)
    {
        return Ok(found);
    }
    let mut waiting = vec![root.to_path_buf()];
    while let Some(dir) = waiting.pop() {
        for entry in backend.read_dir(&dir)? {
            if entry.meta.is_symlink {
                continue;
            }
            if entry.meta.is_dir {
                waiting.push(entry.path);
            } else {
                found.insert(
                    slashed(&entry.path),
                    core::Now {
                        size: entry.meta.len,
                        mtime: entry.meta.modified.and_then(millis),
                    },
                );
            }
        }
    }
    Ok(found)
}

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn millis(at: SystemTime) -> Option<i64> {
    at.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
}

/// How a sync decides, from how it was set up.
fn mode_of(sync: &Sync, now_ms: i64) -> core::Mode {
    core::Mode {
        direction: match sync.direction {
            SyncDirection::Push => core::Direction::Push,
            SyncDirection::Pull => core::Direction::Pull,
            SyncDirection::All => core::Direction::All,
        },
        anchor: sync.anchor.map(|a| a.0),
        first_check: match sync.first_check {
            tungstate_journal::FirstCheck::Full => core::FirstCheck::Full,
            tungstate_journal::FirstCheck::Sampled => core::FirstCheck::Sampled,
            tungstate_journal::FirstCheck::Size => core::FirstCheck::Size,
        },
        cooldown_ms: i64::try_from(sync.cooldown.as_millis()).unwrap_or(i64::MAX),
        now_ms,
        exact: sync.exact,
        on_conflict: match sync.on_conflict {
            ConflictAction::Rename => core::OnConflict::Rename,
            ConflictAction::Skip => core::OnConflict::Skip,
            // `sync add` refuses `replace`; a journal edited by hand gets the
            // default rather than a guess.
            ConflictAction::Quarantine | ConflictAction::Replace => core::OnConflict::Quarantine,
        },
        on_remove: match sync.on_remove {
            tungstate_journal::OnRemove::SetAside => core::OnRemove::SetAside,
            tungstate_journal::OnRemove::Delete => core::OnRemove::Delete,
        },
    }
}

fn now_ms() -> i64 {
    millis(SystemTime::now()).unwrap_or_default()
}

/// A reason a run should not go ahead without a person saying so.
///
/// Data rather than a sentence, so the command line and the window each say
/// it their own way and neither can drift from what is checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A member lists nothing, where after the last run it held `held`
    /// files. An unmounted volume, a mistyped folder and a connection that
    /// lands in the wrong place all look exactly like everything deleted.
    Hollow { member: String, held: usize },
    /// The run would take `taking_off` of `of` files off one member, past the
    /// same limits a reorganisation has.
    Blast {
        member: String,
        taking_off: usize,
        of: usize,
    },
}

/// Everything about a decided run that needs a person's yes.
#[must_use]
pub fn refusals(opened: &Opened, decided: &Decided) -> Vec<Refusal> {
    let mut refused: Vec<Refusal> = decided
        .plan
        .hollow
        .iter()
        .map(|id| Refusal::Hollow {
            member: opened.name(*id).to_string(),
            held: decided.held.get(id).copied().unwrap_or(0),
        })
        .collect();
    for place in &opened.places {
        if let Some(blast) = decided.plan.blast.get(&place.member.id.0)
            && blast.over_limit
        {
            refused.push(Refusal::Blast {
                member: place.member.name.clone(),
                taking_off: blast.removing + blast.replacing,
                of: blast.of,
            });
        }
    }
    refused
}

/// What one leg did.
#[derive(Debug, Clone)]
pub struct LegRan {
    pub from: String,
    pub to: String,
    pub summary: Summary,
    /// Files whose time could not be carried across, because the target
    /// cannot set one. Reported, never a failure.
    pub times_kept: usize,
}

/// Something in a run that did not happen, and why. Tried again next run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missed {
    pub member: String,
    pub path: String,
    pub why: String,
}

/// What a run did.
#[derive(Debug, Clone, Default)]
pub struct Ran {
    /// `None` when there was nothing to do, so nothing was journalled.
    pub plan: Option<PlanId>,
    pub legs: Vec<LegRan>,
    /// Copies set aside or deleted.
    pub taken_off: usize,
    /// Files renamed in place.
    pub renamed: usize,
    /// Removals and renames that did not happen.
    pub missed: Vec<Missed>,
    /// Readings remembered.
    pub remembered: usize,
}

/// Carry a decided run out. Legs run one after another: two legs writing one
/// member at once would race on names.
///
/// # Errors
/// [`SyncError::Leg`] if a leg cannot run at all, or [`SyncError::Vanished`]
/// if a member's volume changed underneath the run. A file that fails is
/// reported in [`Ran`] instead, and simply decided again next run.
#[allow(clippy::too_many_lines)]
pub fn run(
    opened: &Opened,
    decided: &Decided,
    journal: &Journal,
    progress: &mut dyn Progress,
    parallel: Option<usize>,
) -> Result<Ran> {
    let plan = &decided.plan;
    if plan.is_empty() {
        return Ok(Ran::default());
    }
    // What a crash left of a previous run's set-asides and deletes is settled
    // first, so the journal never describes a world that stopped being true.
    let mut pinned = BTreeMap::new();
    for place in &opened.places {
        tungstate_execute::resolve_interrupted_on(
            place.backend.as_ref(),
            journal,
            place.end.connection,
            &place.end.path.to_string_lossy(),
        )
        .map_err(|e| SyncError::Unreadable {
            member: place.member.name.clone(),
            why: e.to_string(),
        })?;
        let token = place
            .backend
            .root_token()
            .map_err(|e| SyncError::Unreachable {
                member: place.member.name.clone(),
                why: e.to_string(),
            })?;
        pinned.insert(place.member.id.0, token);
    }
    let snapshot = decided
        .members
        .iter()
        .map(|m| format!("{}:{}", m.id, m.files.len()))
        .collect::<Vec<_>>()
        .join(",");
    // Written before the first op, so a crash half-way cannot leave a record
    // that claims a run which deleted something can be taken back.
    let id = journal.begin_plan_for(
        &format!("sync:{}", opened.sync.name),
        &snapshot,
        None,
        plan.reversible(),
        Some(Purpose::Sync),
    )?;
    let mut runner = Runner {
        opened,
        decided,
        journal,
        plan: id,
        pinned,
        ran: Ran {
            plan: Some(id),
            ..Ran::default()
        },
        failed: BTreeSet::new(),
        blocked: BTreeSet::new(),
        readings: Vec::new(),
    };

    // 1. Everything taken off or renamed, member by member, before any copy
    //    needs the name.
    for place in &opened.places {
        let member = place.member.id.0;
        for op in &plan.ops {
            match op {
                SyncOp::Remove {
                    member: m,
                    path,
                    how,
                    because,
                } if *m == member && *because != Removed::Moved => {
                    runner.take_off(place, path, how)?;
                    // A replaced version's folder is about to receive the
                    // new one; every other removal may leave a folder empty.
                    if *because != Removed::Superseded {
                        prune(place.backend.as_ref(), path);
                    }
                }
                _ => {}
            }
        }
        for op in &plan.ops {
            if let SyncOp::Rename {
                member: m,
                from,
                to,
                hash,
            } = op
                && *m == member
            {
                runner.rename(place, from, to, hash.as_ref())?;
                prune(place.backend.as_ref(), from);
            }
        }
    }

    // 2. The legs.
    let mut landed = BTreeSet::new();
    for leg in &plan.legs {
        let (outcome, arrived) = runner.leg(leg, progress, parallel)?;
        landed.extend(arrived.into_iter().map(|path| (leg.to, path)));
        runner.ran.legs.push(outcome);
    }

    // 3. The old name of a tidy carried to a member that cannot rename, only
    //    where the new name landed. `carry_move` in core emits each such
    //    removal straight after the copy it depends on, so the last copy seen
    //    to that member is its new name.
    let mut copied_to: BTreeMap<i64, &String> = BTreeMap::new();
    for op in &plan.ops {
        match op {
            SyncOp::Copy { to, path, .. } => {
                copied_to.insert(*to, path);
            }
            SyncOp::Remove {
                member,
                path,
                how,
                because: Removed::Moved,
            } => {
                let arrived = copied_to
                    .get(member)
                    .is_some_and(|fresh| landed.contains(&(*member, (*fresh).clone())));
                if arrived {
                    runner.take_off(opened.place(*member), path, how)?;
                    prune(opened.place(*member).backend.as_ref(), path);
                } else {
                    runner.failed.insert(path.clone());
                }
            }
            _ => {}
        }
    }

    // A path's own readings are remembered only once all of it happened.
    // Remembering the source's new version while a copy of it failed would
    // leave two members that each look unchanged and hold different bytes,
    // which next run would call a conflict instead of finishing the copy.
    let records: Vec<Reading> = plan
        .ops
        .iter()
        .filter_map(|op| match op {
            SyncOp::Record { member, path, seen } if !runner.failed.contains(path) => {
                Some(Reading {
                    member: MemberId(*member),
                    path: path.clone(),
                    present: seen.present,
                    size: seen.size,
                    mtime: seen.mtime,
                    hash: seen
                        .hash
                        .clone()
                        .or_else(|| copied_hash(&runner.readings, path)),
                })
            }
            _ => None,
        })
        .collect();
    let mut readings = std::mem::take(&mut runner.readings);
    readings.extend(records);
    runner.ran.remembered = readings.len();
    journal.record_baseline(&readings, id)?;
    Ok(runner.ran)
}

/// The digest a copy of `path` landed with this run. The member it was sent
/// from never read it, and remembering what the copy computed there too is
/// what lets a later run recognise the file when a tidy moves it.
fn copied_hash(readings: &[Reading], path: &str) -> Option<String> {
    readings
        .iter()
        .find(|r| r.present && r.path == path && r.hash.is_some())
        .and_then(|r| r.hash.clone())
}

/// One run in progress.
struct Runner<'a> {
    opened: &'a Opened,
    decided: &'a Decided,
    journal: &'a Journal,
    plan: PlanId,
    pinned: BTreeMap<i64, RootToken>,
    ran: Ran,
    /// Paths with any part not done: none of their records are written.
    failed: BTreeSet<String>,
    /// (member, path) pairs a copy must not go to, or come from, because
    /// what had to happen there first did not.
    blocked: BTreeSet<(i64, String)>,
    readings: Vec<Reading>,
}

impl Runner<'_> {
    /// Stop if a member's volume is not the one the run started on: writing
    /// to whatever is mounted there now is how files end up on the wrong disk.
    fn still_there(&self, place: &Place) -> Result<()> {
        let now = place.backend.root_token().ok();
        if now.as_ref() == self.pinned.get(&place.member.id.0) {
            Ok(())
        } else {
            Err(SyncError::Vanished {
                member: place.member.name.clone(),
            })
        }
    }

    fn miss(&mut self, place: &Place, path: &str, why: String) {
        self.failed.insert(path.to_string());
        self.blocked.insert((place.member.id.0, path.to_string()));
        self.ran.missed.push(Missed {
            member: place.member.name.clone(),
            path: path.to_string(),
            why,
        });
    }

    /// Whether the file is as it was when the run was decided. Someone may be
    /// writing it right now, and taking it away from under them is the
    /// rudest thing this program could do.
    fn unchanged(&self, place: &Place, path: &str) -> std::result::Result<(), String> {
        let decided = self
            .decided
            .members
            .iter()
            .find(|m| m.id == place.member.id.0)
            .and_then(|m| m.files.get(path));
        let now = place
            .backend
            .stat(Path::new(path))
            .map_err(|_| "it is no longer there".to_string())?;
        match decided {
            Some(was) if was.size == now.len && was.mtime == now.modified.and_then(millis) => {
                Ok(())
            }
            _ => Err("it has been written to since the run was decided".to_string()),
        }
    }

    /// Journal an op under the run, do it, and record how it went.
    fn step(
        &mut self,
        op: &NewOp,
        act: impl FnOnce() -> tungstate_backend::Result<()>,
    ) -> Result<std::result::Result<(), String>> {
        let entry = self.journal.begin(op)?;
        self.journal.attach_to_plan(entry, self.plan)?;
        match act() {
            Ok(()) => {
                self.journal
                    .finish(entry, &Outcome::Committed { hash: None })?;
                Ok(Ok(()))
            }
            Err(error) => {
                let why = explain(&error);
                self.journal
                    .finish(entry, &Outcome::Failed { error: why.clone() })?;
                Ok(Err(why))
            }
        }
    }

    fn take_off(&mut self, place: &Place, path: &str, how: &Removal) -> Result<()> {
        self.still_there(place)?;
        if let Err(why) = self.unchanged(place, path) {
            self.miss(place, path, why);
            return Ok(());
        }
        let size = place.backend.stat(Path::new(path)).ok().map(|m| m.len);
        let outcome = match how {
            Removal::SetAside { to } => {
                // Chosen free when decided, and checked again, because
                // `rename` replaces whatever is at the far end.
                if place.backend.stat(Path::new(to)).is_ok() {
                    self.miss(place, path, format!("`{to}` is already taken"));
                    return Ok(());
                }
                let op = NewOp {
                    kind: OpKind::Rename,
                    source: Some(place.at(path)),
                    destination: Some(place.at(to)),
                    size,
                    link: None,
                    link_id: None,
                };
                self.step(&op, || {
                    if let Some(parent) = Path::new(to).parent() {
                        place.backend.create_dir_all(parent)?;
                    }
                    place.backend.rename(Path::new(path), Path::new(to))
                })?
            }
            Removal::Delete => {
                let op = NewOp {
                    kind: OpKind::Remove,
                    source: Some(place.at(path)),
                    destination: None,
                    size,
                    link: None,
                    link_id: None,
                };
                self.step(&op, || place.backend.remove_file(Path::new(path)))?
            }
        };
        match outcome {
            Ok(()) => {
                self.ran.taken_off += 1;
                self.readings.push(absent(place, path));
            }
            Err(why) => self.miss(place, path, why),
        }
        Ok(())
    }

    fn rename(&mut self, place: &Place, from: &str, to: &str, hash: Option<&String>) -> Result<()> {
        self.still_there(place)?;
        if let Err(why) = self.unchanged(place, from) {
            self.miss(place, from, why);
            self.blocked.insert((place.member.id.0, to.to_string()));
            self.failed.insert(to.to_string());
            return Ok(());
        }
        if place.backend.stat(Path::new(to)).is_ok() {
            self.miss(place, from, format!("`{to}` is already taken"));
            self.blocked.insert((place.member.id.0, to.to_string()));
            self.failed.insert(to.to_string());
            return Ok(());
        }
        let op = NewOp {
            kind: OpKind::Rename,
            source: Some(place.at(from)),
            destination: Some(place.at(to)),
            size: None,
            link: None,
            link_id: None,
        };
        let outcome = self.step(&op, || {
            if let Some(parent) = Path::new(to).parent() {
                place.backend.create_dir_all(parent)?;
            }
            place.backend.rename(Path::new(from), Path::new(to))
        })?;
        match outcome {
            Ok(()) => {
                self.ran.renamed += 1;
                self.readings.push(absent(place, from));
                if let Ok(meta) = place.backend.stat(Path::new(to)) {
                    self.readings.push(Reading {
                        member: place.member.id,
                        path: to.to_string(),
                        present: true,
                        size: meta.len,
                        mtime: meta.modified.and_then(millis),
                        hash: hash.cloned(),
                    });
                }
            }
            Err(why) => {
                self.miss(place, from, why);
                self.blocked.insert((place.member.id.0, to.to_string()));
                self.failed.insert(to.to_string());
            }
        }
        Ok(())
    }

    /// One leg: the files, through the engine, then what landed, remembered.
    /// Returns the source paths that landed.
    fn leg(
        &mut self,
        leg: &core::Leg,
        progress: &mut dyn Progress,
        parallel: Option<usize>,
    ) -> Result<(LegRan, BTreeSet<String>)> {
        let from = self.opened.place(leg.from);
        let to = self.opened.place(leg.to);
        let stopped = |why: String| SyncError::Leg {
            from: from.member.name.clone(),
            to: to.member.name.clone(),
            why,
        };
        self.still_there(to)?;
        let chosen: Vec<PathBuf> = leg
            .paths
            .iter()
            .filter(|path| {
                let landing = leg.parked.get(*path).unwrap_or(path);
                !self.blocked.contains(&(leg.to, landing.clone()))
                    && !self.blocked.contains(&(leg.from, (*path).clone()))
            })
            .map(PathBuf::from)
            .collect();
        let link = leg_link(self.opened, self.journal, from, to)?;
        let since = now_ms();
        // `Skip`, because anything a target held at a path has already been
        // set aside, journalled, in phase 1. Something there now arrived
        // since the run was decided, and is not this run's to replace.
        let mut resolver = FixedResolver(ConflictAction::Skip);
        let landing: BTreeMap<PathBuf, PathBuf> = leg
            .parked
            .iter()
            .map(|(path, parked)| (PathBuf::from(path), PathBuf::from(parked)))
            .collect();
        let mut transfer = Transfer::new(
            &link,
            from.backend.as_ref(),
            to.backend.as_ref(),
            self.journal,
            &mut resolver,
            progress,
        )
        .landing(landing);
        if let Some(at_once) = parallel {
            transfer = transfer.parallel(at_once);
        }
        let summary = transfer
            .run_selection(&chosen)
            .map_err(|e| stopped(e.to_string()))?;

        let times: BTreeMap<&str, Option<i64>> = self
            .decided
            .members
            .iter()
            .find(|m| m.id == leg.from)
            .map(|m| m.files.iter().map(|(p, n)| (p.as_str(), n.mtime)).collect())
            .unwrap_or_default();
        let mut landed = BTreeSet::new();
        let mut times_kept = 0;
        for op in self.journal.landed(link.id, since)? {
            let (Some(source), Some(dest)) = (op.source.as_ref(), op.destination.as_ref()) else {
                continue;
            };
            let source = slashed(&source.path);
            let path = slashed(&dest.path);
            self.journal.attach_to_plan(op.id, self.plan)?;
            if let Some(Some(ms)) = times.get(source.as_str())
                && let Ok(at) = u64::try_from(*ms).map(|ms| UNIX_EPOCH + Duration::from_millis(ms))
                && to
                    .backend
                    .set_modified(Path::new(&path), at)
                    .unwrap_or(false)
            {
                times_kept += 1;
            }
            // A parked version is not part of the set, so nothing is
            // remembered about it.
            if !is_reserved(&path) {
                // What the target reports now, whatever that is: it is only
                // ever compared with itself.
                let meta = to
                    .backend
                    .stat(Path::new(&path))
                    .map_err(|e| stopped(e.to_string()))?;
                self.readings.push(Reading {
                    member: to.member.id,
                    path: path.clone(),
                    present: true,
                    size: meta.len,
                    mtime: meta.modified.and_then(millis),
                    hash: op.hash.clone(),
                });
            }
            landed.insert(source);
        }
        for path in &leg.paths {
            if !landed.contains(path) {
                self.failed.insert(path.clone());
            }
        }
        Ok((
            LegRan {
                from: from.member.name.clone(),
                to: to.member.name.clone(),
                summary,
                times_kept,
            },
            landed,
        ))
    }
}

/// Remove the folders above `path` that are now empty, nearest first, up to
/// the member's root.
///
/// A folder is not something a sync keeps in step, so one emptied by taking
/// its last file off would otherwise linger on one member after it was
/// deleted on another. Best effort and never journalled: `undo` makes a
/// file's folder again on the way back, and a folder that cannot be removed
/// is only untidy.
pub(crate) fn prune(backend: &dyn Backend, path: &str) {
    for folder in Path::new(path).ancestors().skip(1) {
        if folder.as_os_str().is_empty()
            || !backend
                .read_dir(folder)
                .is_ok_and(|entries| entries.is_empty())
            || backend.remove_dir(folder).is_err()
        {
            return;
        }
    }
}

fn absent(place: &Place, path: &str) -> Reading {
    Reading {
        member: place.member.id,
        path: path.to_string(),
        present: false,
        size: 0,
        mtime: None,
        hash: None,
    }
}

/// An error's whole chain, as one sentence.
fn explain(error: &dyn std::error::Error) -> String {
    use std::fmt::Write as _;
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(next) = source {
        let _ = write!(text, ": {next}");
        source = next.source();
    }
    text
}

/// The link a sync owns for one ordered pair, made the first time it is
/// needed and reused after that, so interrupted work is found by link.
fn leg_link(
    opened: &Opened,
    journal: &Journal,
    from: &Place,
    to: &Place,
) -> Result<tungstate_journal::Link> {
    let name = format!(
        "sync:{}:{}>{}",
        opened.sync.id.0, from.member.id.0, to.member.id.0
    );
    if let Ok(mut link) = journal.link_by_name(&name) {
        // The sync's own setting, not the one the leg was made with, so
        // `sync set --verify` reaches legs that already exist. The engine
        // reads it from this copy; the stored row is only ever a starting
        // point.
        link.verify = opened.sync.verify;
        return Ok(link);
    }
    let _: LinkId = journal.create_leg(
        opened.sync.id,
        &NewLink {
            name: name.clone(),
            source: from.end.clone(),
            destination: to.end.clone(),
            source_policy: SourcePolicy::Keep,
            verify: opened.sync.verify,
            order: Order::default(),
            on_conflict: ConflictAction::Skip,
            // The decision already held back anything still being written.
            cooldown: Duration::ZERO,
            saved: false,
        },
    )?;
    Ok(journal.link_by_name(&name)?)
}
