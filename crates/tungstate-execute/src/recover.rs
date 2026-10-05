//! Closing the books on operations that were in flight when the process died.
//!
//! A drain's interrupted operation may have left a half-copied temporary at the
//! destination, so recovering one is a repair. A reorganisation's operations
//! are single renames, which either happened or did not — so recovery here is
//! a *question*, asked of the filesystem, and the answer closes the row
//! honestly. Nothing is retried and no plan is resumed: replanning is cheap and
//! convergent, so the next `plan` simply describes whatever shape the folder is
//! actually in.

use std::path::Path;

use tungstate_backend::Backend;
use tungstate_journal::{ConnectionId, Journal, Location, Op as JournalOp, OpKind, Outcome};

use crate::Result;

/// What was decided about one interrupted operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// The destination exists and the source does not: it happened.
    Committed,
    /// The source is still there: it never happened.
    Abandoned,
    /// Neither side is where it should be. Recorded as failed and left for a
    /// person, because guessing here is guessing about someone's files.
    Unclear,
}

/// Resolve every operation left `intended` for `root`, by asking which side of
/// the rename won.
///
/// Called before applying or undoing anything, so the journal never describes
/// a world that stopped being true while the process was dead.
///
/// # Errors
/// [`crate::ExecuteError::Journal`] if the rows cannot be read or written.
pub fn resolve_interrupted(
    backend: &dyn Backend,
    journal: &Journal,
    root: &str,
) -> Result<Vec<(String, Resolution)>> {
    resolve_where(backend, journal, |op| belongs_to(op, root))
}

/// [`resolve_interrupted`] for one place reached through a connection, or
/// this machine for `None`: a sync's members are several places, and two of
/// them may share a root spelling on different machines.
///
/// # Errors
/// As [`resolve_interrupted`].
pub fn resolve_interrupted_on(
    backend: &dyn Backend,
    journal: &Journal,
    connection: Option<ConnectionId>,
    root: &str,
) -> Result<Vec<(String, Resolution)>> {
    resolve_where(backend, journal, |op| {
        op.source
            .as_ref()
            .or(op.destination.as_ref())
            .is_some_and(|l| l.connection == connection && l.root.to_string_lossy() == root)
    })
}

fn resolve_where(
    backend: &dyn Backend,
    journal: &Journal,
    ours: impl Fn(&JournalOp) -> bool,
) -> Result<Vec<(String, Resolution)>> {
    let mut resolved = Vec::new();
    for op in journal.incomplete()? {
        // A drain's unfinished work belongs to a link and is recovered by the
        // transfer engine, which knows about temporary files. Leave it alone.
        if op.link_id.is_some() || !ours(&op) {
            continue;
        }
        let resolution = look(&op, backend);
        let describe = op
            .source
            .as_ref()
            .map(|l| l.path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        let outcome = match resolution {
            Resolution::Committed => Outcome::Committed { hash: None },
            Resolution::Abandoned => Outcome::Skipped {
                reason: "interrupted before it happened".to_string(),
            },
            Resolution::Unclear => Outcome::Failed {
                error: "interrupted, and neither side is where it should be".to_string(),
            },
        };
        journal.finish(op.id, &outcome)?;
        resolved.push((describe, resolution));
    }
    Ok(resolved)
}

fn belongs_to(op: &JournalOp, root: &str) -> bool {
    op.source
        .as_ref()
        .or(op.destination.as_ref())
        .is_some_and(|l| l.root.to_string_lossy() == root)
}

/// A move by copy that stopped with both files there: finish it if the copy
/// is whole, take it back if not.
fn settle_a_copy(op: &JournalOp, backend: &dyn Backend) -> Resolution {
    let (Some(from), Some(to)) = (op.source.as_ref(), op.destination.as_ref()) else {
        return Resolution::Unclear;
    };
    let (from, to) = (Path::new(&from.path), Path::new(&to.path));
    let (Ok(source), Ok(copy)) = (backend.stat(from), backend.stat(to)) else {
        return Resolution::Unclear;
    };
    if copy.len == source.len {
        if backend.remove_file(from).is_ok() {
            Resolution::Committed
        } else {
            Resolution::Unclear
        }
    } else if backend.remove_file(to).is_ok() {
        Resolution::Abandoned
    } else {
        Resolution::Unclear
    }
}

/// Ask the filesystem which side of the rename won.
fn look(op: &JournalOp, backend: &dyn Backend) -> Resolution {
    let exists =
        |path: Option<&Location>| path.is_some_and(|l| backend.stat(Path::new(&l.path)).is_ok());
    match op.kind {
        OpKind::MkDir => {
            if exists(op.source.as_ref()) {
                Resolution::Committed
            } else {
                Resolution::Abandoned
            }
        }
        // A directory removed, or a sync's outright delete: still there
        // means it never happened.
        OpKind::RmDir | OpKind::Remove => {
            if exists(op.source.as_ref()) {
                Resolution::Abandoned
            } else {
                Resolution::Committed
            }
        }
        OpKind::Rename | OpKind::Move => {
            match (exists(op.source.as_ref()), exists(op.destination.as_ref())) {
                // The rename went through and the source is gone.
                (false, true) => Resolution::Committed,
                // It never started.
                (true, false) => Resolution::Abandoned,
                // Where there is no rename, a move is a copy and then a
                // delete, so both is exactly what stopping between them
                // leaves. A whole copy finishes the move; a short one goes.
                (true, true) if !backend.capabilities().atomic_rename => settle_a_copy(op, backend),
                // Both, or neither. A rename cannot leave both, so this is
                // someone else's doing -- or the file is simply missing, which
                // is not something to guess about.
                _ => Resolution::Unclear,
            }
        }
        OpKind::Copy => Resolution::Unclear,
    }
}
