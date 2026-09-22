//! Carrying out a plan, and taking one back.
//!
//! Slice 6 said what would happen and changed nothing. This is where every
//! safety rail in DESIGN.md §4 stops being a note and becomes code: the intent
//! is journalled before the filesystem is touched, the folder is checked
//! against what the plan believed, and every operation can be reversed.
//!
//! The ordering is the drain's, for the drain's reason: journal intent, act,
//! journal outcome. What is different here is recovery. A drain's interrupted
//! operation may have left a half-copied temporary; a reorganisation's are
//! single renames, which either happened or did not. So recovery is a question
//! — *which side of the rename won?* — rather than a repair, and **no plan is
//! ever resumed**. Replanning is cheap and convergent, so continuing a
//! half-applied plan buys nothing and risks acting on a stale view.

pub mod digest;
mod recover;
mod undo;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use tungstate_backend::{Backend, BackendError, RootToken};
use tungstate_core::plan::{Op, Plan};
use tungstate_core::snapshot;
use tungstate_journal::plans::PlanId;
use tungstate_journal::{Journal, JournalError, Location, NewOp, OpKind, Outcome};

pub use recover::{Resolution, resolve_interrupted};
pub use undo::{Undone, invert, undo};

/// Anything that stops a plan being carried out.
#[derive(Debug, thiserror::Error)]
pub enum ExecuteError {
    /// The folder is not as the plan believed, found before anything was
    /// attempted. Nothing has been changed.
    ///
    /// DESIGN §4: *stale plan → replan, not stale action.*
    #[error("the folder has changed since this plan was made: {what}")]
    Stale {
        /// What was different.
        what: String,
    },

    /// The folder stopped being as the plan believed part-way through.
    ///
    /// Carries what was done, because the answer to "what now" depends on it:
    /// those operations are journalled under `plan` and can be taken back.
    #[error("stopped after {done} operation(s): {what}")]
    StoppedPartWay {
        /// The reorganisation, for `undo --plan`.
        plan: PlanId,
        /// How many operations had succeeded.
        done: usize,
        /// What stopped it.
        what: String,
    },

    /// The root is not the volume it was. A vanished mount must never be
    /// silently recreated; this is the invariant that exists because one once
    /// put 572 MB on the wrong disk and deleted the originals.
    #[error("`{root}` is not the volume this plan was made against")]
    RootChanged {
        /// The folder.
        root: String,
    },

    /// The backend refused.
    #[error("could not {doing}")]
    Backend {
        /// What was being attempted.
        doing: String,
        /// Why it failed.
        #[source]
        source: BackendError,
    },

    /// The journal refused.
    #[error(transparent)]
    Journal(#[from] JournalError),
}

/// Result alias so signatures read `Result<Applied>`.
pub type Result<T> = std::result::Result<T, ExecuteError>;

/// What carrying out a plan actually did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// The reorganisation's id, which is what `undo --plan` takes.
    pub plan: PlanId,
    /// Operations that succeeded.
    pub done: usize,
    /// Operations skipped because the file had changed under us.
    pub skipped: Vec<Skipped>,
    /// Operations that were attempted and refused.
    pub failed: Vec<Failed>,
    /// Operations never reached, because the run stopped.
    pub remaining: usize,
}

impl Applied {
    /// Whether everything the plan asked for happened.
    #[must_use]
    pub fn complete(&self) -> bool {
        self.skipped.is_empty() && self.failed.is_empty() && self.remaining == 0
    }
}

/// One operation left alone because the file moved under us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The file.
    pub path: String,
    /// Why.
    pub why: String,
}

/// One operation that was attempted and refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed {
    /// The file or directory.
    pub path: String,
    /// Why.
    pub why: String,
}

/// Carry out `plan` against `backend`, recording every step in `journal`.
///
/// `fresh` is a snapshot taken immediately before this call; its fingerprint
/// must match the one the plan was built from, or the folder has moved on and
/// the plan is guesswork. That check is what makes a saved `plan.json` safe to
/// apply later.
///
/// The circuit breaker is **not** applied here. Whether a blast radius or a
/// policy that never settles should stop the run is a question for whoever is
/// talking to the user; this function does what it is told, and the CLI is
/// where `--yes` lives.
///
/// # Errors
/// [`ExecuteError::Stale`] if the folder has changed since the plan was made,
/// [`ExecuteError::RootChanged`] if the volume is not the one planned against,
/// or [`ExecuteError::Journal`] if the record cannot be written. A failure to
/// carry out one operation is reported in [`Applied`] rather than returned.
pub fn apply(
    plan: &Plan,
    fresh: &tungstate_core::Snapshot,
    backend: &dyn Backend,
    journal: &Journal,
    root: &str,
) -> Result<Applied> {
    let planned = tungstate_core::plan::fingerprint(fresh);
    if planned != plan.snapshot {
        return Err(ExecuteError::Stale {
            what: "a file has been added, removed or written to since it was planned".to_string(),
        });
    }

    // Pinned once and checked before every operation, exactly as a drain pins
    // its destination. A governed folder on an external drive is the same risk
    // a vanished NAS mount is, wearing different clothes.
    let token: RootToken = backend
        .root_token()
        .map_err(|source| ExecuteError::Backend {
            doing: format!("read `{root}`"),
            source,
        })?;

    // A plan that hands anything to the desktop's trash can never be taken
    // back, and the flag has to be written before the first op: a crash
    // half-way through must not leave a record that claims to be reversible.
    let reversible = !plan.ops.iter().any(|op| matches!(op, Op::Trash { .. }));
    let id = journal.begin_plan_that(root, &plan.snapshot, None, reversible)?;
    let mut applied = Applied {
        plan: id,
        done: 0,
        skipped: Vec::new(),
        failed: Vec::new(),
        remaining: 0,
    };

    for (index, op) in plan.ops.iter().enumerate() {
        if !same_volume(backend, &token) {
            applied.remaining = plan.ops.len() - index;
            return Err(ExecuteError::StoppedPartWay {
                plan: id,
                done: applied.done,
                what: format!("`{root}` is not the volume this plan was made against"),
            });
        }

        match step(op, fresh, backend, journal, root, id) {
            Ok(Step::Done) => applied.done += 1,
            Ok(Step::Skipped(skipped)) => applied.skipped.push(skipped),
            Ok(Step::Failed(failed)) => applied.failed.push(failed),
            // The world is not as the plan believed, so everything after this
            // is guesswork. Stop, and let the caller replan -- which is cheap,
            // and correct, because a half-applied folder is exactly what the
            // next plan describes.
            Err(error) => {
                applied.remaining = plan.ops.len() - index;
                tracing::warn!(?error, done = applied.done, "stopping a plan part-way");
                return Err(ExecuteError::StoppedPartWay {
                    plan: id,
                    done: applied.done,
                    what: describe_error(&error),
                });
            }
        }
    }

    Ok(applied)
}

/// An error's whole chain, as one sentence.
fn describe_error(error: &ExecuteError) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(next) = source {
        use std::fmt::Write as _;
        let _ = write!(text, ": {next}");
        source = next.source();
    }
    text
}

/// How one operation turned out.
enum Step {
    Done,
    Skipped(Skipped),
    Failed(Failed),
}

/// Carry out one operation, journalling the intent first.
fn step(
    op: &Op,
    planned: &tungstate_core::Snapshot,
    backend: &dyn Backend,
    journal: &Journal,
    root: &str,
    plan: PlanId,
) -> Result<Step> {
    // The second staleness check, and the narrower one. Someone may be editing
    // this file right now; moving it out from under them is the rudest thing
    // this program could do.
    if let Some(from) = op.source()
        && let Some(why) = changed_since(from, planned, backend)
    {
        return Ok(Step::Skipped(Skipped {
            path: from.to_string(),
            why,
        }));
    }

    let id = journal.begin(&NewOp {
        kind: kind_of(op),
        source: op
            .source()
            .or_else(|| op.directory())
            .map(|p| Location::new(root, p)),
        destination: op.target().map(|p| Location::new(root, p)),
        size: op.source().and_then(|p| size_of(p, planned)),
        link: None,
        link_id: None,
    })?;
    journal.attach_to_plan(id, plan)?;

    match perform(op, backend) {
        Ok(()) => {
            journal.finish(id, &Outcome::Committed { hash: None })?;
            Ok(Step::Done)
        }
        Err(error) => {
            let why = describe(&error);
            journal.finish(id, &Outcome::Failed { error: why.clone() })?;
            if fatal(&error) {
                // "The destination is occupied" and "the source has gone" are
                // statements about the plan, not about this file.
                return Err(ExecuteError::Stale { what: why });
            }
            // "Permission denied" is about this file alone. DESIGN §9: never
            // abort the whole cycle for one bad directory.
            Ok(Step::Failed(Failed {
                path: op
                    .source()
                    .or_else(|| op.directory())
                    .unwrap_or("")
                    .to_string(),
                why,
            }))
        }
    }
}

/// Do the thing. The only function in the crate that changes a filesystem.
fn perform(op: &Op, backend: &dyn Backend) -> std::result::Result<(), BackendError> {
    match op {
        Op::MkDir { path } => backend.create_dir_all(Path::new(path)),
        Op::RmDir { path } => backend.remove_dir(Path::new(path)),
        Op::Move { from, to, .. } => backend.rename(Path::new(from), Path::new(to)),
        Op::Quarantine { from, to, .. } => {
            // Quarantine makes its own directory on the way, the way the drain
            // does. A plan does not spell that out, because where a parked
            // file goes is not a decision anyone reviews.
            if let Some(parent) = Path::new(to).parent()
                && parent != Path::new("")
            {
                backend.create_dir_all(parent)?;
            }
            backend.rename(Path::new(from), Path::new(to))
        }
        Op::Trash { path, .. } => {
            // The desktop trash is this machine's. A backend that cannot say
            // where a file really is has no trash to put it in, and refusing
            // is the only honest answer: a remote trash is DESIGN §4's
            // `.tungstate-trash/`, and is not built.
            let Some(full) = backend.on_this_machine(Path::new(path))? else {
                return Err(BackendError::Io {
                    path: PathBuf::from(path),
                    source: std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "the trash is this machine's; set aside instead",
                    ),
                });
            };
            trash::delete(&full).map_err(|source| BackendError::Io {
                path: full,
                source: std::io::Error::other(source.to_string()),
            })
        }
    }
}

/// Whether this failure says the *plan* is wrong rather than this one file.
fn fatal(error: &BackendError) -> bool {
    match error {
        // The root is gone, or a path escaped. Neither gets better by trying
        // the next operation.
        BackendError::RootUnreachable(_)
        | BackendError::PathEscapesRoot(_)
        | BackendError::PathNotRelative(_) => true,
        BackendError::Io { source, .. } => matches!(
            source.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::AlreadyExists
        ),
        _ => false,
    }
}

fn describe(error: &BackendError) -> String {
    use std::fmt::Write as _;
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(next) = source {
        let _ = write!(text, ": {next}");
        source = next.source();
    }
    text
}

/// Whether the file at `path` is still as the snapshot found it.
fn changed_since(
    path: &str,
    planned: &tungstate_core::Snapshot,
    backend: &dyn Backend,
) -> Option<String> {
    let expected = planned
        .entries
        .iter()
        .find(|entry| entry.relative_path() == path)?;
    let Ok(now) = backend.stat(Path::new(path)) else {
        // Gone entirely. Left for `perform` to report, so there is one place
        // that decides whether a missing source is fatal.
        return None;
    };
    if now.len != expected.size {
        return Some(format!(
            "it was {} bytes when planned and is {} now",
            expected.size, now.len
        ));
    }
    // Compared as `SystemTime`, so this crate needs no clock library of its
    // own: the snapshot's timestamp came from the same `Meta::modified`.
    let planned_mtime = expected.mtime;
    let actual_mtime = now.modified.and_then(to_timestamp);
    if planned_mtime != actual_mtime {
        return Some("it has been written to since it was planned".to_string());
    }
    None
}

/// `Meta::modified` as the snapshot spells it, so the two can be compared.
fn to_timestamp(time: std::time::SystemTime) -> Option<jiff::Timestamp> {
    jiff::Timestamp::try_from(time).ok()
}

fn size_of(path: &str, planned: &tungstate_core::Snapshot) -> Option<u64> {
    planned
        .entries
        .iter()
        .find(|entry| entry.relative_path() == path)
        .map(|entry| entry.size)
}

/// The journal's vocabulary for one of the planner's operations.
fn kind_of(op: &Op) -> OpKind {
    match op {
        Op::MkDir { .. } => OpKind::MkDir,
        Op::RmDir { .. } => OpKind::RmDir,
        // A quarantine is a rename, and calling it one keeps `undo` from
        // needing a fifth case for a move that is spelled differently.
        Op::Move { .. } | Op::Quarantine { .. } => OpKind::Rename,
        // The journal's word for "it is not there any more", which is as
        // much as this side can say: where the trash put it belongs to the
        // desktop, and `undo` refuses a plan that used it.
        Op::Trash { .. } => OpKind::Remove,
    }
}

fn same_volume(backend: &dyn Backend, pinned: &RootToken) -> bool {
    backend.root_token().is_ok_and(|now| &now == pinned)
}

/// Where a parked file goes, so the CLI and the planner agree.
#[must_use]
pub fn quarantine_dir() -> &'static str {
    snapshot::QUARANTINE
}
