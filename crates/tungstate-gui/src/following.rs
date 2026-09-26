//! Syncs kept in step while the window is open: slice 9e.
//!
//! `tungstate_sync::follow` decides when a sync is due; this runs it, the
//! same way Run does, through the same gate and the same queue, so a sync
//! kept in step never runs beside a transfer or beside another sync. What it
//! does is reported as `sync://…` events like any other run, plus the state
//! of each member (`sync://following`) and a sync that stopped to ask
//! (`sync://held`).
//!
//! A run that would remove anything, or needs a yes, does not happen here:
//! the sync is held, and the window asks. It is kept in step again once a
//! person has run it or said so.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tungstate_journal::{Journal, Launch};
use tungstate_sync::follow::{self, Handle, Heard, Outcome, Pace, State};

use crate::sync::{Ask, Guard, decided, run_decided, work};
use crate::{App, lock};

/// One member's state, as its status line draws it.
#[derive(Debug, Clone, Serialize)]
pub struct MemberStatus {
    pub sync: String,
    pub member: String,
    /// `watching`, `polling` or `paused`.
    pub state: &'static str,
    /// For `polling`: how often, in seconds.
    pub every_secs: Option<u64>,
    /// For `paused`: why, as a sentence.
    pub why: Option<String>,
}

/// Everything the window shows about syncs kept in step.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FollowingView {
    pub members: Vec<MemberStatus>,
    /// Syncs that stopped to ask, by name.
    pub held: Vec<String>,
}

/// The loop in flight, if any, and what it has said.
#[derive(Default)]
pub struct Following {
    handle: Mutex<Option<Arc<Handle>>>,
    status: Mutex<BTreeMap<(String, String), MemberStatus>>,
    held: Mutex<BTreeSet<String>>,
    /// The last trouble said about each sync, so a member out of reach is
    /// reported once rather than on every retry.
    said: Mutex<BTreeMap<String, String>>,
}

impl Following {
    pub fn view(&self) -> FollowingView {
        FollowingView {
            members: lock(&self.status).values().cloned().collect(),
            held: lock(&self.held).iter().cloned().collect(),
        }
    }

    /// Whether this trouble is news: not the same thing said last time.
    fn news(&self, sync: &str, why: &str) -> bool {
        lock(&self.said)
            .insert(sync.to_string(), why.to_string())
            .as_deref()
            != Some(why)
    }

    /// A person has looked at a held sync: keep it in step again.
    pub fn resume(&self, sync: &str) {
        lock(&self.held).remove(sync);
        if let Some(handle) = lock(&self.handle).as_ref() {
            handle.resume(sync);
        }
    }

    fn halt(&self) {
        if let Some(handle) = lock(&self.handle).take() {
            handle.stop();
        }
        lock(&self.status).clear();
    }
}

fn status(sync: &str, member: &str, state: &State) -> MemberStatus {
    let (word, every_secs, why) = match state {
        State::Watching => ("watching", None, None),
        State::Polling { every } => ("polling", Some(every.as_secs()), None),
        State::Paused { why } => ("paused", None, Some(why.clone())),
    };
    MemberStatus {
        sync: sync.to_string(),
        member: member.to_string(),
        state: word,
        every_secs,
        why,
    }
}

/// Start keeping every sync marked `continuous` in step, stopping whatever
/// was doing so before. Called when the window opens and whenever a sync is
/// made, changed or removed, so the loop always follows what the journal says.
pub fn restart(app: &AppHandle) {
    let state = app.state::<App>();
    state.following.halt();
    let wanted: Vec<String> = match state.journal.syncs() {
        Ok(syncs) => syncs
            .into_iter()
            .filter(|s| s.launch == Launch::Continuous)
            .map(|s| s.name)
            .collect(),
        Err(_) => return,
    };
    if wanted.is_empty() {
        let _ = app.emit("sync://following", state.following.view());
        return;
    }
    let handle = Handle::new();
    *lock(&state.following.handle) = Some(Arc::clone(&handle));
    let worker = app.clone();
    std::thread::spawn(move || {
        let state = worker.state::<App>();
        let secrets = crate::secrets();
        let mut followed = Vec::new();
        for name in &wanted {
            match tungstate_sync::open(&state.journal, name, &secrets) {
                Ok(opened) => followed.push(follow::followed(opened)),
                // A sync with a member out of reach at start is said to be
                // paused, and followed again the next time anything about
                // syncs changes or the window opens.
                Err(error) => {
                    let paused = MemberStatus {
                        sync: name.clone(),
                        member: String::new(),
                        state: "paused",
                        every_secs: None,
                        why: Some(crate::describe(error)),
                    };
                    lock(&state.following.status).insert((name.clone(), String::new()), paused);
                }
            }
        }
        let _ = worker.emit("sync://following", state.following.view());
        let mut told = |heard: &Heard| {
            match heard {
                Heard::Member {
                    sync,
                    member,
                    state: now,
                } => {
                    lock(&state.following.status)
                        .insert((sync.clone(), member.clone()), status(sync, member, now));
                }
                Heard::Held { sync } => {
                    lock(&state.following.held).insert(sync.clone());
                }
                Heard::Running { .. } => {}
            }
            let _ = worker.emit("sync://following", state.following.view());
        };
        let mut run = |name: &str| step(&worker, &state.journal, name);
        if let Err(why) = follow::follow(followed, &Pace::default(), &handle, &mut told, &mut run) {
            tracing::warn!(%why, "could not keep syncs in step");
        }
    });
}

/// One run the loop asked for.
fn step(app: &AppHandle, journal: &Journal, name: &str) -> Outcome {
    let state = app.state::<App>();
    {
        // The same gate a transfer and a pressed Run take: one decision about
        // who may write, so nothing runs beside anything else.
        let _gate = lock(&state.gate);
        if state.queue.is_running() || !state.syncing.queue.claim() {
            return Outcome::Busy;
        }
    }
    let guard = Guard::new(app.clone());
    let outcome = claimed(app, journal, name);
    // Whatever was pressed while this ran is picked up here, and `work` stops
    // being the worker once the queue is empty.
    work(app);
    guard.disarm();
    outcome
}

/// The run itself, with the queue held.
fn claimed(app: &AppHandle, journal: &Journal, name: &str) -> Outcome {
    let state = app.state::<App>();
    let (opened, decided) = match decided(journal, name, Ask::default()) {
        Ok(pair) => pair,
        Err(message) => {
            if state.following.news(name, &message) {
                let _ = app.emit(
                    "sync://error",
                    crate::sync::SyncErrorEvent {
                        sync: name.to_string(),
                        kind: "failed",
                        message,
                    },
                );
            }
            return Outcome::Busy;
        }
    };
    lock(&state.following.said).remove(name);
    let plan = &decided.plan;
    if !follow::unattended(&opened, &decided) {
        return Outcome::Held;
    }
    let unsettled = follow::unsettled(&decided);
    if plan.is_empty() {
        return Outcome::Done {
            wrote: Vec::new(),
            unsettled,
        };
    }
    let wrote = follow::written(&opened, plan);
    state.syncing.stop.clear();
    let _ = match run_decided(app, journal, &state.syncing.stop, name, &opened, &decided) {
        Ok(ran) => app.emit("sync://done", ran),
        Err(message) => app.emit(
            "sync://error",
            crate::sync::SyncErrorEvent {
                sync: name.to_string(),
                kind: "failed",
                message,
            },
        ),
    };
    Outcome::Done { wrote, unsettled }
}
