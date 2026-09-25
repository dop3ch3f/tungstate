//! Taking a reorganisation back.
//!
//! The inverse of each operation is obvious and total, which is the whole
//! reason the op set was kept to four: `Move(a → b)` reverses to
//! `Move(b → a)`, `MkDir(d)` to `RmDir(d)`, and each the other way. Applied in
//! **reverse order**, so the last directory created is the first removed.
//!
//! Undo is *checked, not forced*. Before reversing a move, the file must still
//! be where the journal left it and its old name must still be free. Forcing
//! would overwrite whatever has since been put there, and an undo that
//! destroys work is worse than no undo at all.

use std::path::Path;

use tungstate_backend::Backend;
use tungstate_core::plan::{Because, Op};
use tungstate_journal::plans::PlanId;
use tungstate_journal::{Journal, Location, NewOp, Op as JournalOp, OpKind, OpStatus, Outcome};

use crate::{ExecuteError, Result};

/// What taking a plan back actually did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undone {
    /// The reorganisation that was reversed.
    pub plan: PlanId,
    /// Operations put back.
    pub done: usize,
}

/// The operations that would undo `ops`, in the order they must run.
///
/// Pure, so the hard part — that reversing is reverse order, not just reversed
/// operations — is testable with no filesystem.
#[must_use]
pub fn invert(ops: &[JournalOp]) -> Vec<Op> {
    ops.iter()
        .rev()
        // Only what actually happened. A failed or skipped operation changed
        // nothing, so undoing it would be inventing work.
        .filter(|op| op.status == OpStatus::Committed)
        .filter_map(|op| {
            let source = op.source.as_ref().map(|l| path_of(&l.path));
            let destination = op.destination.as_ref().map(|l| path_of(&l.path));
            match (op.kind, source, destination) {
                (OpKind::Rename | OpKind::Move, Some(from), Some(to)) => Some(Op::Move {
                    from: to,
                    to: from,
                    because: Because::Rule {
                        name: "undo".to_string(),
                    },
                }),
                (OpKind::MkDir, Some(path), _) => Some(Op::RmDir { path }),
                (OpKind::RmDir, Some(path), _) => Some(Op::MkDir { path }),
                // A drain's copy or removal is not a reorganisation and cannot
                // appear under a plan id. Ignored rather than guessed at.
                _ => None,
            }
        })
        .collect()
}

/// One step of taking a sync run back, on whichever member it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reversal {
    /// A copy the run delivered: taken off again, but only if it still holds
    /// what was delivered.
    Unsend {
        at: Location,
        hash: Option<String>,
        size: Option<u64>,
    },
    /// A rename the run made, set-asides included: moved back.
    Unrename { now_at: Location, back_to: Location },
}

/// A sync run that deleted files outright, which nothing can put back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Irreversible {
    pub deleted: usize,
}

/// The steps that would take a sync run back, in the order they must run.
///
/// Beside [`invert`] rather than inside it: a reorganisation happens in one
/// folder and reverses to the planner's own operations, while a sync run
/// spans every member and reverses copies as well as renames.
///
/// # Errors
/// [`Irreversible`] if the run deleted anything outright.
pub fn invert_sync(ops: &[JournalOp]) -> std::result::Result<Vec<Reversal>, Irreversible> {
    let committed = || ops.iter().filter(|op| op.status == OpStatus::Committed);
    let deleted = committed().filter(|op| op.kind == OpKind::Remove).count();
    if deleted > 0 {
        return Err(Irreversible { deleted });
    }
    Ok(committed()
        .rev()
        .filter_map(|op| match (op.kind, &op.source, &op.destination) {
            (OpKind::Copy, _, Some(at)) => Some(Reversal::Unsend {
                at: at.clone(),
                hash: op.hash.clone(),
                size: op.size,
            }),
            (OpKind::Rename | OpKind::Move, Some(from), Some(to)) => Some(Reversal::Unrename {
                now_at: to.clone(),
                back_to: from.clone(),
            }),
            _ => None,
        })
        .collect())
}

fn path_of(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Reverse a reorganisation.
///
/// # Errors
/// [`ExecuteError::Stale`] if the folder has moved on in a way that makes a
/// reversal unsafe, or [`ExecuteError::Journal`] if the plan is unknown or has
/// already been taken back.
pub fn undo(
    plan: PlanId,
    fresh: &tungstate_core::Snapshot,
    backend: &dyn Backend,
    journal: &Journal,
    root: &str,
) -> Result<Undone> {
    let recorded = journal.plan_by_id(plan)?;
    if !recorded.reversible {
        return Err(ExecuteError::Journal(
            tungstate_journal::JournalError::PlanIrreversible(plan.0),
        ));
    }
    if recorded.is_undone() {
        return Err(ExecuteError::Journal(
            tungstate_journal::JournalError::PlanAlreadyUndone(plan.0),
        ));
    }

    let ops = invert(&journal.ops_for_plan(plan)?);

    // Refused as a whole, before anything moves, by asking slice 6's paper
    // model whether this order is executable against the folder as it is now.
    // The same question `plan` asks of its own operations, and the same code,
    // so an undo cannot be judged by a rule the plan was not.
    //
    // Checked up front rather than step by step because finding out half way
    // leaves the folder in a third shape nobody asked for.
    tungstate_core::plan::replay(&ops, fresh).map_err(|error| ExecuteError::Stale {
        what: error.to_string(),
    })?;

    let id = journal.begin_plan(root, &format!("undo of plan {}", plan.0), Some(plan))?;
    let mut done = 0;
    for op in &ops {
        let entry = journal.begin(&NewOp {
            kind: crate::kind_of(op),
            source: op
                .source()
                .or_else(|| op.directory())
                .map(|p| Location::new(root, p)),
            destination: op.target().map(|p| Location::new(root, p)),
            size: None,
            link: None,
            link_id: None,
        })?;
        journal.attach_to_plan(entry, id)?;

        match crate::perform(op, backend) {
            Ok(()) => {
                journal.finish(entry, &Outcome::Committed { hash: None })?;
                done += 1;
            }
            Err(error) => {
                let why = crate::describe(&error);
                journal.finish(entry, &Outcome::Failed { error: why.clone() })?;
                // An undo that half-works and carries on is worse than one
                // that stops: the folder is now neither shape. Stop, and say
                // exactly where.
                return Err(ExecuteError::Stale { what: why });
            }
        }
    }

    journal.mark_undone(plan)?;
    Ok(Undone { plan, done })
}
