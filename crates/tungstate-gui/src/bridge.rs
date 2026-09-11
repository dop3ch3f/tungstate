//! Adapters between the synchronous engine and the asynchronous window.
//!
//! The engine runs on its own thread and blocks freely. The window never
//! blocks. Everything here exists to let those two live together.

use std::path::Path;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tungstate_journal::ConflictAction;
use tungstate_transfer::{Conflict, ConflictResolver, FileOutcome, Progress, RunShape, SkipReason};

/// What the run turned out to be, before any of it happens.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct BeganEvent {
    /// False for a copy, and the window words everything differently for it.
    pub removes_originals: bool,
    /// Files in flight. Shown, because "is it going one at a time?" was a
    /// question the window previously gave no way to answer.
    pub at_once: usize,
}

/// One file the run intends to deal with, in the order it will take them.
#[derive(Debug, Clone, Serialize)]
pub struct PlannedEvent {
    pub path: String,
    pub size: u64,
}

/// Bytes moved for the file in flight. Throttled by the engine.
#[derive(Debug, Clone, Serialize)]
pub struct AdvancedEvent {
    pub path: String,
    pub done: u64,
    pub total: u64,
}

/// A file starting to transfer.
#[derive(Debug, Clone, Serialize)]
pub struct StartedEvent {
    pub path: String,
    pub size: u64,
}

/// A file that has been dealt with.
#[derive(Debug, Clone, Serialize)]
pub struct FinishedEvent {
    pub path: String,
    /// One of transferred, already-present, skipped, quarantined, failed.
    pub outcome: &'static str,
    /// Present when the outcome needs explaining, such as a skip.
    pub detail: Option<&'static str>,
}

/// A conflict the window must decide about.
#[derive(Debug, Clone, Serialize)]
pub struct ConflictEvent {
    pub path: String,
    pub incoming_size: u64,
    pub existing_size: u64,
}

/// Forwards engine progress into the window as events.
pub struct EventProgress {
    app: AppHandle,
}

impl EventProgress {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl Progress for EventProgress {
    fn began(&mut self, shape: RunShape) {
        let _ = self.app.emit(
            "transfer://began",
            BeganEvent {
                removes_originals: shape.removes_originals,
                at_once: shape.at_once,
            },
        );
    }

    fn planned(&mut self, files: &[tungstate_transfer::Planned]) {
        let _ = self.app.emit(
            "transfer://planned",
            files
                .iter()
                .map(|f| PlannedEvent {
                    path: f.path.display().to_string(),
                    size: f.size,
                })
                .collect::<Vec<_>>(),
        );
    }

    fn advanced(&mut self, path: &Path, done: u64, total: u64) {
        let _ = self.app.emit(
            "transfer://advanced",
            AdvancedEvent {
                path: path.display().to_string(),
                done,
                total,
            },
        );
    }

    fn starting(&mut self, path: &Path, size: u64) {
        // A failed emit means the window has gone. The engine keeps working;
        // losing a progress line is not a reason to abandon a drain.
        let _ = self.app.emit(
            "transfer://started",
            StartedEvent {
                path: path.display().to_string(),
                size,
            },
        );
    }

    fn finished(&mut self, path: &Path, outcome: FileOutcome) {
        let (outcome, detail) = match outcome {
            FileOutcome::Transferred => ("transferred", None),
            FileOutcome::AlreadyPresent => ("already-present", None),
            FileOutcome::Quarantined => ("quarantined", None),
            FileOutcome::Failed => ("failed", None),
            FileOutcome::Skipped(SkipReason::RecentlyModified) => (
                "skipped",
                Some("written too recently; it will move on the next run"),
            ),
            FileOutcome::Skipped(SkipReason::Conflict) => {
                ("skipped", Some("another file holds that name"))
            }
        };
        let _ = self.app.emit(
            "transfer://finished",
            FinishedEvent {
                path: path.display().to_string(),
                outcome,
                detail,
            },
        );
    }
}

/// What the window sent back when asked about a conflict.
#[derive(Debug, Clone, Copy)]
pub struct Reply {
    pub action: ConflictAction,
    /// Apply to every later conflict in this run without asking again.
    pub apply_to_all: bool,
}

/// The half of the conflict channel the window writes to.
///
/// Held in Tauri's state so the `resolve_conflict` command can reach it.
#[derive(Default)]
pub struct ConflictChannel(Mutex<Option<Sender<Reply>>>);

impl ConflictChannel {
    /// Open a channel for one run, returning the end the engine waits on.
    pub fn open(&self) -> Receiver<Reply> {
        let (tx, rx) = channel();
        *self.lock() = Some(tx);
        rx
    }

    /// Close it, so a stale reply from a dismissed dialog cannot be delivered.
    pub fn close(&self) {
        *self.lock() = None;
    }

    /// Deliver the window's answer. False if nothing is waiting for one.
    pub fn reply(&self, reply: Reply) -> bool {
        self.lock()
            .as_ref()
            .is_some_and(|tx| tx.send(reply).is_ok())
    }

    /// A poisoned lock here means a panic while swapping a channel end. The
    /// data is a plain Option and is still sound, and refusing every later
    /// conflict would strand a running drain.
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Sender<Reply>>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Asks the window what to do, and blocks the engine thread until it answers.
///
/// Blocking is correct here: the engine has its own thread, and the alternative
/// is inventing a suspended-transfer state for a question that is nearly always
/// answered in seconds.
pub struct WindowResolver {
    app: AppHandle,
    replies: Receiver<Reply>,
    /// Set once the user answers "do this for all of them".
    sticky: Option<ConflictAction>,
    /// Used when the window never answers.
    fallback: ConflictAction,
}

impl WindowResolver {
    pub fn new(app: AppHandle, replies: Receiver<Reply>, fallback: ConflictAction) -> Self {
        Self {
            app,
            replies,
            sticky: None,
            fallback,
        }
    }
}

/// How long a conflict dialog waits before giving up on the user.
///
/// Long enough to walk away and come back, short enough that a closed window
/// cannot strand a drain overnight. On timeout the link's configured action
/// applies, which is the same thing an unattended run would have done.
const ANSWER_TIMEOUT: Duration = Duration::from_mins(15);

impl ConflictResolver for WindowResolver {
    fn resolve(&mut self, conflict: &Conflict) -> ConflictAction {
        if let Some(action) = self.sticky {
            return action;
        }

        let sent = self.app.emit(
            "transfer://conflict",
            ConflictEvent {
                path: conflict.path.display().to_string(),
                incoming_size: conflict.incoming_size,
                existing_size: conflict.existing_size,
            },
        );
        if sent.is_err() {
            // No window to ask.
            return self.fallback;
        }

        let Ok(reply) = self.replies.recv_timeout(ANSWER_TIMEOUT) else {
            tracing::warn!("no answer to a conflict; applying the link's configured action");
            return self.fallback;
        };

        if reply.apply_to_all {
            self.sticky = Some(reply.action);
        }
        reply.action
    }
}
