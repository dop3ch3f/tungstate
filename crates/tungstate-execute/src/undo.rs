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
