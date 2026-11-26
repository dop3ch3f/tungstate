//! Adapters between the synchronous engine and the asynchronous window.
//!
//! The engine runs on its own thread and blocks freely. The window never
//! blocks. Everything here exists to let those two live together.

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tungstate_journal::ConflictAction;

use crate::keepalive::{Errand, Keepalive};
use crate::ledger::Ledger;
use tungstate_transfer::{
    Conflict, ConflictResolver, FileOutcome, Identical, IdenticalAction, Original, Progress,
    RunShape, SkipReason, Stop,
};

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

/// A file whose content is already byte-for-byte at the destination, where
/// the link would otherwise remove the original from this machine.
#[derive(Debug, Clone, Serialize)]
pub struct IdenticalEvent {
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
    /// `transfer` or `sync`: the two runs are shown in different sections
    /// and may be going at once, so each has its own stream of events.
    stream: &'static str,
    /// The same events, kept for a window shown again and for a phone's
    /// notification. Transfers only.
    ledger: Option<Arc<Mutex<Ledger>>>,
    /// Keeps a phone's transfer going in the background, while one runs.
    keepalive: Option<Keepalive>,
}

impl EventProgress {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            stream: "transfer",
            ledger: None,
            keepalive: None,
        }
    }

    /// The same events, as `sync://…`.
    pub fn for_sync(app: AppHandle) -> Self {
        Self {
            app,
            stream: "sync",
            ledger: None,
            keepalive: None,
        }
    }

    /// Keep every event in `ledger` as well.
    #[must_use]
    pub fn with_ledger(mut self, ledger: Arc<Mutex<Ledger>>) -> Self {
        self.ledger = Some(ledger);
        self
    }

    /// Keep the phone's transfer going for `errand`, or retitle the one
    /// already kept going when the queue moves to the next transfer.
    pub fn keep_alive(&mut self, stop: &Arc<Stop>, errand: Errand) {
        match &mut self.keepalive {
            Some(keep) => keep.retitle(errand),
            None => self.keepalive = Some(Keepalive::begin(&self.app, Arc::clone(stop), errand)),
        }
    }

    /// Tells the notification how far, at most once a second unless `now`.
    pub fn pulse(&mut self, now: bool) {
        let (Some(keep), Some(ledger)) = (&mut self.keepalive, &self.ledger) else {
            return;
        };
        let (counts, waiting) = {
            let ledger = lock_ledger(ledger);
            (ledger.counts(), ledger.waiting().is_some())
        };
        keep.pulse(&counts, waiting, now);
    }

    fn keep(&mut self, change: impl FnOnce(&mut Ledger)) {
        if let Some(ledger) = &self.ledger {
            change(&mut lock_ledger(ledger));
        }
        self.pulse(false);
    }

    fn emit<P: Serialize + Clone>(&self, event: &str, payload: P) {
        // A failed emit means the window has gone. The engine keeps working;
        // losing a progress line is not a reason to abandon a run.
        let _ = self
            .app
            .emit(&format!("{}://{event}", self.stream), payload);
    }
}

impl Progress for EventProgress {
    fn began(&mut self, shape: RunShape) {
        self.emit(
            "began",
            BeganEvent {
                removes_originals: shape.removes_originals,
                at_once: shape.at_once,
            },
        );
    }

    fn planned(&mut self, files: &[tungstate_transfer::Planned]) {
        self.keep(|l| l.planned(files.iter().map(|f| (f.path.display().to_string(), f.size))));
        self.emit(
            "planned",
            files
                .iter()
                .map(|f| PlannedEvent {
                    path: f.path.display().to_string(),
                    size: f.size,
                })
                .collect::<Vec<_>>(),
        );
    }

    fn at_once(&mut self, files: usize) {
        self.emit("at-once", files);
    }

    fn checking(&mut self, path: &Path, done: u64, total: u64) {
        self.keep(|l| l.checking(&path.display().to_string(), done, total));
        self.emit(
            "checking",
            AdvancedEvent {
                path: path.display().to_string(),
                done,
                total,
            },
        );
    }

    fn advanced(&mut self, path: &Path, done: u64, total: u64) {
        self.keep(|l| l.advanced(&path.display().to_string(), done, total));
        self.emit(
            "advanced",
            AdvancedEvent {
                path: path.display().to_string(),
                done,
                total,
            },
        );
    }

    fn starting(&mut self, path: &Path, size: u64) {
        self.keep(|l| l.starting(&path.display().to_string(), size));
        self.emit(
            "started",
            StartedEvent {
                path: path.display().to_string(),
                size,
            },
        );
    }

    fn finished(&mut self, path: &Path, outcome: FileOutcome) {
        let (outcome, detail) = match outcome {
            FileOutcome::Transferred => ("transferred", None),
            FileOutcome::AlreadyPresent(Original::Removed) => (
                "already-present",
                Some("both copies read and compared; the original here was removed"),
            ),
            FileOutcome::AlreadyPresent(Original::Kept) => (
                "already-present",
                Some("both copies read and compared; the original here was kept"),
            ),
            FileOutcome::Carried => (
                "already-present",
                Some("sent and checked earlier in this copy, before it was interrupted"),
            ),
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
        self.keep(|l| l.finished(&path.display().to_string(), outcome, detail));
        self.emit(
            "finished",
            FinishedEvent {
                path: path.display().to_string(),
                outcome,
                detail,
            },
        );
    }
}

/// A poisoned ledger only means a panic mid-update; what it holds is still the
/// best account there is of the run.
pub fn lock_ledger(ledger: &Mutex<Ledger>) -> std::sync::MutexGuard<'_, Ledger> {
    ledger
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What the window sent back when asked about a conflict.
#[derive(Debug, Clone, Copy)]
pub struct Reply {
    pub answer: Answer,
    /// Apply to every later question of the same kind in this run.
    pub apply_to_all: bool,
}

/// Which question the window is answering.
///
/// One channel carries both because only one question is ever outstanding —
/// the engine holds a single resolver behind a lock — and the variant is what
/// stops a stale answer to one being read as an answer to the other.
#[derive(Debug, Clone, Copy)]
pub enum Answer {
    /// What to do about something already holding the name.
    Conflict(ConflictAction),
    /// What to do with the original when the destination already matches.
    Identical(IdenticalAction),
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
    /// The same, for the identical-copy question. Separate because answering
    /// every conflict one way says nothing about whether originals may go.
    sticky_identical: Option<IdenticalAction>,
    /// What the run was told to do about a clash.
    ///
    /// Doing double duty on purpose: it is the answer when the window never
    /// replies, *and* the reason not to ask in the first place unless it is
    /// `Quarantine`, which is the one the dialog labels "ask me".
    fallback: ConflictAction,
}

impl WindowResolver {
    /// Take this link's rule for clashes.
    ///
    /// The window's queue can grow while it is running, so there is no first
    /// link whose rule can speak for every link after it. A sticky answer the
    /// user gave is deliberately left alone: "do this for all of them" was
    /// said about the run, not about one leg of it.
    pub fn follow_conflict_rule(&mut self, rule: ConflictAction) {
        self.fallback = rule;
    }

    pub fn new(app: AppHandle, replies: Receiver<Reply>, fallback: ConflictAction) -> Self {
        Self {
            app,
            replies,
            sticky: None,
            sticky_identical: None,
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

        // A choice already made is not a question. The rule lives on
        // `ConflictAction` so the window and the command line cannot come to
        // different conclusions about the same setting.
        if !self.fallback.wants_asking() {
            return self.fallback;
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

        let Answer::Conflict(action) = reply.answer else {
            tracing::warn!(
                "an answer to a different question arrived; using the configured action"
            );
            return self.fallback;
        };
        if reply.apply_to_all {
            self.sticky = Some(action);
        }
        action
    }

    fn identical(&mut self, found: &Identical) -> IdenticalAction {
        if let Some(action) = self.sticky_identical {
            return action;
        }

        let sent = self.app.emit(
            "transfer://identical",
            IdenticalEvent {
                path: found.path.display().to_string(),
                size: found.size,
            },
        );
        // No window to ask. Keep it: deleting somebody's only other copy
        // because nobody was there to say no is the outcome this exists to
        // prevent, and a file left behind costs disk space rather than data.
        if sent.is_err() {
            return IdenticalAction::KeepOriginal;
        }

        let Ok(reply) = self.replies.recv_timeout(ANSWER_TIMEOUT) else {
            tracing::warn!("no answer about an identical copy; leaving the original here");
            return IdenticalAction::KeepOriginal;
        };

        let Answer::Identical(action) = reply.answer else {
            return IdenticalAction::KeepOriginal;
        };
        if reply.apply_to_all {
            self.sticky_identical = Some(action);
        }
        action
    }
}
