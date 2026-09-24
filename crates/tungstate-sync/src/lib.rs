//! Running a sync.
//!
//! [`open`] reaches every member, [`decide`] surveys them and asks
//! `tungstate_core::sync` what should move (reading whatever bytes it asks
//! for), and [`run`] carries each leg through the transfer engine and
//! remembers what landed. Three steps because a preview is the first two and
//! changes nothing.
//!
//! A leg is an ordinary link the sync owns, so every rail the drain has earned
//! comes with it: temp names, verify before commit, resume after a crash, and
//! the destination pinned so a vanished volume stops the run.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tungstate_backend::Backend;
use tungstate_core::dupes::Digest as _;
use tungstate_core::sync::{self as core, Digests, SAMPLED, SyncOp, SyncPlan};
use tungstate_journal::{
    ConflictAction, Endpoint, Journal, LinkId, MemberId, NewLink, Order, PlanId, Purpose, Reading,
    SourcePolicy, Sync, SyncDirection, ends,
};
use tungstate_secret::SecretStore;
use tungstate_transfer::{FixedResolver, Progress, Summary, Transfer};

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
/// [`SyncError::NotYet`] for a setting 9b does not carry out yet, or
/// [`SyncError::Unreachable`] naming the member that could not be reached.
pub fn open(journal: &Journal, name: &str, secrets: &dyn SecretStore) -> Result<Opened> {
    let sync = journal.sync_by_name(name)?;
    if sync.exact {
        return Err(SyncError::NotYet(
            "--exact arrives in slice 9c; until then a sync only ever adds".into(),
        ));
    }
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
}

/// Survey every member and decide, reading what the decision asks to read.
///
/// # Errors
/// [`SyncError::Unreadable`] if a member cannot be listed, or
/// [`SyncError::Read`] if a file the decision needs cannot be read.
///
/// # Panics
/// Never in practice: the probe policy is a constant that has its own test.
pub fn decide(opened: &Opened, journal: &Journal) -> Result<Decided> {
    let probe = tungstate_core::policy::Policy::parse(tungstate_core::learn::PROBE)
        .expect("the probe policy parses")
        .policy;
    let mut members = Vec::with_capacity(opened.places.len());
    for place in &opened.places {
        let snapshot = tungstate_attrs::survey(place.backend.as_ref(), &probe).map_err(|e| {
            SyncError::Unreadable {
                member: place.member.name.clone(),
                why: e.to_string(),
            }
        })?;
        members.push(core::Member {
            id: place.member.id.0,
            name: place.member.name.clone(),
            local: place.member.connection.is_none() && !place.networked,
            connection: place.member.connection.map(|c| c.0),
            case_sensitive: snapshot.case_sensitive,
            files: core::Member::listing(&snapshot),
        });
    }

    let baseline: core::Baseline = journal
        .baseline_for(opened.sync.id)?
        .into_iter()
        .map(|r| {
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

    let mut digests = Digests::new();
    let (mut read, mut sampled, mut read_for_no_times) = (0, 0, 0);
    // Two rounds settle any path: the first asks for what it needs, the second
    // decides with it. The bound is there so a bug cannot become a hang.
    for _ in 0..4 {
        let plan = core::decide(&members, &baseline, &digests, &mode);
        if plan.questions.is_empty() {
            return Ok(Decided {
                plan,
                members,
                digests,
                read,
                sampled,
                read_for_no_times,
            });
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
            read += 1;
            sampled += usize::from(question.sampled);
            read_for_no_times += usize::from(question.because == core::Because::NoTimes);
            digests.insert((question.member, question.path.clone()), answer);
        }
    }
    Err(SyncError::NotYet(
        "deciding kept asking for more reads; this is a bug, and nothing was moved".into(),
    ))
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
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
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

/// What a run did.
#[derive(Debug, Clone, Default)]
pub struct Ran {
    /// `None` when there was nothing to do, so nothing was journalled.
    pub plan: Option<PlanId>,
    pub legs: Vec<LegRan>,
    /// Readings remembered.
    pub remembered: usize,
}

/// Carry a decided run out. Legs run one after another: two legs writing one
/// member at once would race on names.
///
/// # Errors
/// [`SyncError::Leg`] if a leg cannot run at all. A file that fails inside a
/// leg is reported in its summary instead, and simply decided again next run.
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
    let snapshot = decided
        .members
        .iter()
        .map(|m| format!("{}:{}", m.id, m.files.len()))
        .collect::<Vec<_>>()
        .join(",");
    let id = journal.begin_plan_for(
        &format!("sync:{}", opened.sync.name),
        &snapshot,
        None,
        true,
        Some(Purpose::Sync),
    )?;

    let mut ran = Ran {
        plan: Some(id),
        ..Ran::default()
    };
    let mut failed: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for leg in &plan.legs {
        let (outcome, readings, missed) =
            run_leg(opened, decided, journal, id, leg, progress, parallel)?;
        failed.extend(missed);
        ran.remembered += readings.len();
        journal.record_baseline(&readings, id)?;
        ran.legs.push(outcome);
    }

    // A path's own readings are remembered only once every copy of it landed.
    // Remembering the source's new version while a copy of it failed would
    // leave two members that each look unchanged and hold different bytes,
    // which next run would call a conflict instead of finishing the copy.
    let records: Vec<Reading> = plan
        .ops
        .iter()
        .filter_map(|op| match op {
            SyncOp::Record { member, path, seen } if !failed.contains(path) => Some(Reading {
                member: MemberId(*member),
                path: path.clone(),
                present: seen.present,
                size: seen.size,
                mtime: seen.mtime,
                hash: seen.hash.clone(),
            }),
            _ => None,
        })
        .collect();
    ran.remembered += records.len();
    journal.record_baseline(&records, id)?;
    Ok(ran)
}

/// One leg: the files, through the engine, then what landed, remembered.
fn run_leg(
    opened: &Opened,
    decided: &Decided,
    journal: &Journal,
    plan: PlanId,
    leg: &core::Leg,
    progress: &mut dyn Progress,
    parallel: Option<usize>,
) -> Result<(LegRan, Vec<Reading>, Vec<String>)> {
    let from = opened.place(leg.from);
    let to = opened.place(leg.to);
    let stopped = |why: String| SyncError::Leg {
        from: from.member.name.clone(),
        to: to.member.name.clone(),
        why,
    };
    let link = leg_link(opened, journal, from, to)?;
    let since = now_ms();
    let chosen: Vec<PathBuf> = leg.paths.iter().map(PathBuf::from).collect();
    // `Replace`, because what a target already holds at a path is a version
    // the decision has chosen to supersede. The engine sets it aside first,
    // so an edit arriving never costs the version it replaces.
    let mut resolver = FixedResolver(ConflictAction::Replace);
    let mut transfer = Transfer::new(
        &link,
        from.backend.as_ref(),
        to.backend.as_ref(),
        journal,
        &mut resolver,
        progress,
    );
    if let Some(at_once) = parallel {
        transfer = transfer.parallel(at_once);
    }
    let summary = transfer
        .run_selection(&chosen)
        .map_err(|e| stopped(e.to_string()))?;

    let times: BTreeMap<&str, Option<i64>> = decided
        .members
        .iter()
        .find(|m| m.id == leg.from)
        .map(|m| m.files.iter().map(|(p, n)| (p.as_str(), n.mtime)).collect())
        .unwrap_or_default();
    let mut readings = Vec::new();
    let mut landed = std::collections::BTreeSet::new();
    let mut times_kept = 0;
    for op in journal.landed(link.id, since)? {
        let Some(dest) = op.destination.as_ref() else {
            continue;
        };
        let path = dest.path.to_string_lossy().replace('\\', "/");
        journal.attach_to_plan(op.id, plan)?;
        if let Some(Some(ms)) = times.get(path.as_str())
            && let Ok(at) = u64::try_from(*ms).map(|ms| UNIX_EPOCH + Duration::from_millis(ms))
            && to
                .backend
                .set_modified(Path::new(&path), at)
                .unwrap_or(false)
        {
            times_kept += 1;
        }
        // What the target reports now, whatever that is: it is only ever
        // compared with itself.
        let meta = to
            .backend
            .stat(Path::new(&path))
            .map_err(|e| stopped(e.to_string()))?;
        readings.push(Reading {
            member: to.member.id,
            path: path.clone(),
            present: true,
            size: meta.len,
            mtime: meta.modified.and_then(|t| {
                t.duration_since(UNIX_EPOCH)
                    .ok()
                    .and_then(|d| i64::try_from(d.as_millis()).ok())
            }),
            hash: op.hash.clone(),
        });
        landed.insert(path);
    }
    let missed = leg
        .paths
        .iter()
        .filter(|p| !landed.contains(p.as_str()))
        .cloned()
        .collect();
    Ok((
        LegRan {
            from: from.member.name.clone(),
            to: to.member.name.clone(),
            summary,
            times_kept,
        },
        readings,
        missed,
    ))
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
    if let Ok(link) = journal.link_by_name(&name) {
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
            on_conflict: ConflictAction::Replace,
            // The decision already held back anything still being written.
            cooldown: Duration::ZERO,
            saved: false,
        },
    )?;
    Ok(journal.link_by_name(&name)?)
}
