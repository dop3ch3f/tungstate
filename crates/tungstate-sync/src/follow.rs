//! Keeping syncs in step while something is running to keep them: slice 9e.
//!
//! This is a trigger, not a second engine (DESIGN §4d). It only answers
//! *when* a sync is worth running, and hands the run to whoever called it,
//! who runs it the same way a person pressing Run would. The window is the
//! caller today; the daemon (slice 10) will be the next.
//!
//! - **Members on this machine are watched**, with the same settle rule the
//!   folder watcher uses: every event pushes the run out by the sync's
//!   cooldown, so a file still being written keeps putting it off.
//! - **Members elsewhere are listed**, because no event arrives from another
//!   machine: every 30 seconds while they are changing, doubling each quiet
//!   look up to 10 minutes. One that cannot be reached is said to be paused
//!   once, not once per look, and is tried again at 1, 2, 5, then every 10
//!   minutes until it is back.
//! - **A run's own writes are not news.** Every path a run is about to write
//!   is expected back for a while and ignored when it arrives, and every
//!   member elsewhere is listed again after a run, so what the run wrote
//!   there is the new starting point rather than a change.
//! - **The hour is the truth.** Watchers drop events, so every sync is run
//!   on the hour whatever was heard.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::RecursiveMode;
use notify_debouncer_full::new_debouncer;
use tungstate_core::sync::{Removal, SyncOp, SyncPlan, is_partial};
use tungstate_watch::{Echoes, Schedule, is_ours, root_of, spellings};

use crate::{Decided, Opened};

/// How often to look, and for how long to expect a run's own writes.
///
/// Data rather than constants so a test can run the whole loop in seconds.
#[derive(Debug, Clone)]
pub struct Pace {
    /// How often a member elsewhere is listed while it is changing.
    pub quick: Duration,
    /// How rarely it is listed once it has been quiet for a while.
    pub slowest: Duration,
    /// How long to wait before trying an unreachable member again: the first
    /// entry after the first failure, and so on, the last one for ever after.
    pub retries: Vec<Duration>,
    /// How often every sync is run regardless of what was heard.
    pub sweep: Duration,
    /// How long a run's own writes are expected back.
    pub echo: Duration,
    /// How long to wait before trying a run that could not start because
    /// something else was running.
    pub busy: Duration,
    /// How long a burst of events is gathered before it is looked at.
    pub debounce: Duration,
    /// How often the loop wakes regardless, so a stop is answered promptly.
    pub tick: Duration,
}

impl Default for Pace {
    fn default() -> Self {
        let minutes = |n: u64| Duration::from_secs(n * 60);
        Self {
            quick: Duration::from_secs(30),
            slowest: minutes(10),
            retries: vec![minutes(1), minutes(2), minutes(5), minutes(10)],
            sweep: minutes(60),
            echo: Duration::from_secs(10),
            busy: Duration::from_secs(15),
            debounce: Duration::from_secs(1),
            tick: Duration::from_millis(250),
        }
    }
}

/// How a member is kept an eye on.
pub enum How {
    /// A folder on this machine, watched for events.
    Here(PathBuf),
    /// Somewhere else, listed. The closure answers with something that changes
    /// whenever the member's contents do (a digest of every path, size and
    /// time), or why it could not be listed.
    Listed(Box<dyn FnMut() -> Result<String, String> + Send>),
}

/// One member of a followed sync.
pub struct Member {
    pub name: String,
    pub how: How,
}

/// A sync kept in step.
pub struct Followed {
    pub sync: String,
    /// How long a member must have been quiet before a run.
    pub cooldown: Duration,
    pub members: Vec<Member>,
}

/// What is happening to one member, for a status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Watched for events.
    Watching,
    /// Listed, this often at the moment.
    Polling { every: Duration },
    /// Could not be reached. Tried again on its own.
    Paused { why: String },
}

/// Something the loop did or saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Heard {
    /// A member's state changed. Said once per change, never once per look.
    Member {
        sync: String,
        member: String,
        state: State,
    },
    /// A sync is being run.
    Running { sync: String },
    /// A sync would remove files or needs a yes, and waits for a person. It
    /// is not run again until [`Handle::resume`].
    Held { sync: String },
}

/// What running a sync came to, as the caller reports it.
pub enum Outcome {
    /// It ran, or there was nothing to do. `wrote` is every local path it
    /// wrote, from [`written`], so the events it caused are not taken as news.
    /// `unsettled` when it left a file alone for still being written: no
    /// event will say when that file stops changing, so the sync is run
    /// again once the cooldown has passed.
    Done {
        wrote: Vec<PathBuf>,
        unsettled: bool,
    },
    /// It would remove something or needs a yes. Held until resumed.
    Held,
    /// Something else was running, or a member could not be reached. Tried
    /// again shortly.
    Busy,
}

/// The loop's controls, shared with whoever started it.
#[derive(Debug, Default)]
pub struct Handle {
    stop: std::sync::atomic::AtomicBool,
    resumed: Mutex<Vec<String>>,
}

impl Handle {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// End the loop. It finishes a run in flight first.
    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn stopping(&self) -> bool {
        self.stop.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// A held sync may be kept in step again: a person has looked at it.
    pub fn resume(&self, sync: &str) {
        lock(&self.resumed).push(sync.to_string());
    }

    fn take_resumed(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.resumed))
    }
}

/// Take a lock, ignoring poisoning: a panic elsewhere must not stop a sync
/// being resumed.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One member elsewhere: when to list it next, and what it said last time.
///
/// Pure, so the pace and the pausing can be tested without a network.
#[derive(Debug, Clone)]
pub struct Poll {
    every: Duration,
    next: Instant,
    last: Option<String>,
    failures: usize,
    paused: bool,
}

/// What one listing came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Looked {
    /// Different from last time: worth a run.
    Changed,
    /// The same, or the first listing, which is only a starting point.
    Same,
    /// Could not be listed, for the first time since it last could.
    Paused(String),
    /// Still cannot be listed. Nothing to say.
    StillPaused,
    /// Reachable again. Worth a run: it may have changed while away.
    Back,
}

impl Poll {
    /// A member first listed straight away, as a starting point.
    #[must_use]
    pub fn new(now: Instant, pace: &Pace) -> Self {
        Self {
            every: pace.quick,
            next: now,
            last: None,
            failures: 0,
            paused: false,
        }
    }

    #[must_use]
    pub fn due(&self, now: Instant) -> bool {
        self.next <= now
    }

    #[must_use]
    pub fn next(&self) -> Instant {
        self.next
    }

    #[must_use]
    pub fn every(&self) -> Duration {
        self.every
    }

    /// Take in one listing and decide when the next one is.
    pub fn looked(&mut self, now: Instant, pace: &Pace, listing: Result<String, String>) -> Looked {
        match listing {
            Ok(said) => {
                let was = self.paused;
                self.paused = false;
                self.failures = 0;
                let looked = if was {
                    self.every = pace.quick;
                    Looked::Back
                } else if self.last.as_ref().is_some_and(|last| *last != said) {
                    self.every = pace.quick;
                    Looked::Changed
                } else {
                    // Quiet, or a first look: slow down, up to the slowest.
                    if self.last.is_some() {
                        self.every = (self.every * 2).min(pace.slowest);
                    }
                    Looked::Same
                };
                self.last = Some(said);
                self.next = now + self.every;
                looked
            }
            Err(why) => {
                let wait = pace
                    .retries
                    .get(self.failures)
                    .or(pace.retries.last())
                    .copied()
                    .unwrap_or(pace.slowest);
                self.failures += 1;
                self.next = now + wait;
                if std::mem::replace(&mut self.paused, true) {
                    Looked::StillPaused
                } else {
                    Looked::Paused(why)
                }
            }
        }
    }

    /// What a run just wrote is the new starting point, not a change.
    pub fn rebase(&mut self, listing: Result<String, String>) {
        if let Ok(said) = listing {
            self.last = Some(said);
        }
    }
}

/// Every local path a run of `plan` may write, in every spelling an event
/// might use, with the folders they sit in. What the loop expects back.
#[must_use]
pub fn written(opened: &Opened, plan: &SyncPlan) -> Vec<PathBuf> {
    let mut touched: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
    let mut at = |member: i64, path: &str| {
        touched.entry(member).or_default().insert(path.to_string());
    };
    for op in &plan.ops {
        match op {
            SyncOp::Copy { to, path, .. } => at(*to, path),
            SyncOp::Park { to, parked, .. } => at(*to, parked),
            SyncOp::Remove {
                member, path, how, ..
            } => {
                at(*member, path);
                if let Removal::SetAside { to } = how {
                    at(*member, to);
                }
            }
            SyncOp::Rename {
                member, from, to, ..
            } => {
                at(*member, from);
                at(*member, to);
            }
            SyncOp::Record { .. } => {}
        }
    }
    let mut out = BTreeSet::new();
    for place in &opened.places {
        if place.member.connection.is_some() || place.networked {
            continue;
        }
        let Some(paths) = touched.get(&place.member.id.0) else {
            continue;
        };
        for root in spellings(Path::new(&place.member.path)) {
            let root = PathBuf::from(root);
            for path in paths {
                // The file, and every folder above it that a run may make or
                // prune on the way.
                let mut here = Some(Path::new(path));
                while let Some(part) = here.filter(|p| !p.as_os_str().is_empty()) {
                    out.insert(root.join(part));
                    here = part.parent();
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Keep `followed` in step until the handle says stop. Blocking: the caller
/// gives it a thread. `run` is called for each sync that is due, one at a
/// time, and says what came of it.
///
/// Every sync is run once as soon as this starts: files that changed while
/// nothing was following them generated no event anybody heard.
///
/// # Errors
/// If the platform's watcher cannot be set up at all.
#[allow(clippy::too_many_lines)]
pub fn follow(
    mut followed: Vec<Followed>,
    pace: &Pace,
    handle: &Handle,
    told: &mut dyn FnMut(&Heard),
    run: &mut dyn FnMut(&str) -> Outcome,
) -> Result<(), String> {
    let (sender, events) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(pace.debounce, None, sender).map_err(|e| e.to_string())?;

    // Every spelling of every watched root, and which syncs it belongs to: a
    // folder can be a member of several.
    let mut owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut cooldown: BTreeMap<String, Duration> = BTreeMap::new();
    let mut polls: Vec<(String, String, Poll)> = Vec::new();
    let now = Instant::now();
    for sync in &followed {
        cooldown.insert(sync.sync.clone(), sync.cooldown);
        for member in &sync.members {
            let state = match &member.how {
                How::Here(root) => match debouncer.watch(root, RecursiveMode::Recursive) {
                    Ok(()) => {
                        for spelling in spellings(root) {
                            owners
                                .entry(spelling)
                                .or_default()
                                .insert(sync.sync.clone());
                        }
                        State::Watching
                    }
                    // A folder that cannot be watched is still run on the
                    // hour, and the reason is said once.
                    Err(error) => State::Paused {
                        why: format!("cannot watch it, so it is only checked on the hour: {error}"),
                    },
                },
                How::Listed(_) => {
                    polls.push((sync.sync.clone(), member.name.clone(), Poll::new(now, pace)));
                    State::Polling { every: pace.quick }
                }
            };
            told(&Heard::Member {
                sync: sync.sync.clone(),
                member: member.name.clone(),
                state,
            });
        }
    }
    let roots: Vec<String> = owners.keys().cloned().collect();
    let mut schedule = Schedule::default();
    let mut echoes = Echoes::default();
    let mut held: BTreeSet<String> = BTreeSet::new();
    for sync in &followed {
        schedule.stirred(&sync.sync, now, Duration::ZERO);
    }
    let mut next_sweep = now + pace.sweep;

    let listing = |followed: &mut Vec<Followed>, sync: &str, member: &str| {
        followed
            .iter_mut()
            .find(|f| f.sync == sync)
            .and_then(|f| f.members.iter_mut().find(|m| m.name == member))
            .map_or_else(
                || Err("no longer followed".to_string()),
                |m| match &mut m.how {
                    How::Listed(list) => list(),
                    How::Here(_) => Err("watched, not listed".to_string()),
                },
            )
    };

    while !handle.stopping() {
        let now = Instant::now();
        for sync in handle.take_resumed() {
            if held.remove(&sync) {
                schedule.stirred(&sync, now, Duration::ZERO);
            }
        }

        let next_poll = polls.iter().map(|(_, _, p)| p.next()).min();
        let mut wait = pace.tick.min(next_sweep.saturating_duration_since(now));
        if let Some(quiet) = schedule.quiet_for(now) {
            wait = wait.min(quiet);
        }
        if let Some(at) = next_poll {
            wait = wait.min(at.saturating_duration_since(now));
        }
        if let Ok(result) = events.recv_timeout(wait) {
            let now = Instant::now();
            let paths: Vec<PathBuf> = match result {
                Ok(batch) => batch.into_iter().flat_map(|e| e.event.paths).collect(),
                // The watcher lost track: everything is worth a look.
                Err(_) => Vec::new(),
            };
            if paths.is_empty() {
                for sync in &followed {
                    schedule.stirred(&sync.sync, now, sync.cooldown);
                }
            }
            for path in paths {
                let Some(root) = root_of(&path, &roots) else {
                    continue;
                };
                let partial = path
                    .file_name()
                    .is_some_and(|name| is_partial(&name.to_string_lossy()));
                if path == Path::new(root)
                    || partial
                    || is_ours(&path, root)
                    || echoes.ours(&path, now)
                {
                    continue;
                }
                for sync in owners.get(root).into_iter().flatten() {
                    if !held.contains(sync) {
                        schedule.stirred(sync, now, cooldown[sync]);
                    }
                }
            }
        }

        let now = Instant::now();
        echoes.forget_old(now);

        for (sync, member, poll) in &mut polls {
            if !poll.due(now) {
                continue;
            }
            let said = listing(&mut followed, sync, member);
            let before = poll.every();
            let looked = poll.looked(now, pace, said);
            if matches!(looked, Looked::Changed | Looked::Back) && !held.contains(sync.as_str()) {
                schedule.stirred(sync, now, cooldown[sync.as_str()]);
            }
            // Said when something a person would want to know changed: it
            // paused, it came back, or it is being listed at a new pace.
            // Never once per look.
            let state = match looked {
                Looked::Paused(why) => Some(State::Paused { why }),
                Looked::Back => Some(State::Polling {
                    every: poll.every(),
                }),
                Looked::Changed | Looked::Same if poll.every() != before => Some(State::Polling {
                    every: poll.every(),
                }),
                _ => None,
            };
            if let Some(state) = state {
                told(&Heard::Member {
                    sync: sync.clone(),
                    member: member.clone(),
                    state,
                });
            }
        }

        if now >= next_sweep {
            for sync in &followed {
                if !held.contains(&sync.sync) {
                    schedule.stirred(&sync.sync, now, Duration::ZERO);
                }
            }
            next_sweep = now + pace.sweep;
        }

        for sync in schedule.ready(now) {
            if held.contains(&sync) || handle.stopping() {
                continue;
            }
            told(&Heard::Running { sync: sync.clone() });
            match run(&sync) {
                Outcome::Done { wrote, unsettled } => {
                    if unsettled {
                        schedule.stirred(&sync, Instant::now(), cooldown[&sync]);
                    }
                    let until = Instant::now() + pace.echo;
                    echoes.expect(
                        wrote.into_iter().map(|p| p.to_string_lossy().to_string()),
                        until,
                    );
                    // What the run wrote elsewhere is the new starting point.
                    for (owner, member, poll) in &mut polls {
                        if *owner == sync {
                            let said = listing(&mut followed, owner, member);
                            poll.rebase(said);
                        }
                    }
                }
                Outcome::Held => {
                    held.insert(sync.clone());
                    told(&Heard::Held { sync });
                }
                Outcome::Busy => schedule.stirred(&sync, Instant::now(), pace.busy),
            }
        }
    }
    drop(debouncer);
    Ok(())
}

/// Whether a decided run left a file alone for still being written, so it
/// should be decided again once that file has had time to settle.
#[must_use]
pub fn unsettled(decided: &Decided) -> bool {
    decided
        .plan
        .left_alone
        .iter()
        .any(|left| matches!(left.why, tungstate_core::sync::Why::TooRecent { .. }))
}

/// Whether a run nobody pressed may go ahead: it takes nothing off any
/// member, needs no yes, and deletes nothing outright. Otherwise the sync is
/// held until a person looks. Replacing an old version with a newer one sets
/// the old one aside, like any run, and is fine.
#[must_use]
pub fn unattended(opened: &Opened, decided: &Decided) -> bool {
    let plan = &decided.plan;
    plan.reversible()
        && !plan.blast.values().any(|b| b.removing > 0)
        && crate::refusals(opened, decided).is_empty()
}

/// A sync, reached, as the loop follows it: folders on this machine watched,
/// the rest listed through the backend each was opened with.
#[must_use]
pub fn followed(opened: Opened) -> Followed {
    let sync = opened.sync.name.clone();
    let cooldown = opened.sync.cooldown;
    let members = opened
        .places
        .into_iter()
        .map(|place| {
            let here = place.member.connection.is_none() && !place.networked;
            let how = if here {
                How::Here(PathBuf::from(&place.member.path))
            } else {
                let backend = place.backend;
                How::Listed(Box::new(move || listing(backend.as_ref())))
            };
            Member {
                name: place.member.name,
                how,
            }
        })
        .collect();
    Followed {
        sync,
        cooldown,
        members,
    }
}

/// A digest of every file a member holds, by path, size and time. Changes
/// whenever its contents do, and costs a listing rather than a read.
///
/// # Errors
/// Why the member could not be listed, as a sentence.
pub fn listing(backend: &dyn tungstate_backend::Backend) -> Result<String, String> {
    let mut hasher = blake3::Hasher::new();
    let mut waiting = vec![PathBuf::new()];
    let mut lines = Vec::new();
    while let Some(dir) = waiting.pop() {
        let entries = backend.read_dir(&dir).map_err(|e| crate::explain(&e))?;
        for entry in entries {
            let path = entry.path.to_string_lossy().replace('\\', "/");
            let name = entry
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if tungstate_core::snapshot::is_reserved(&path) || is_partial(&name) {
                continue;
            }
            if entry.meta.is_dir {
                waiting.push(entry.path);
            } else {
                let at = entry
                    .meta
                    .modified
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_millis());
                lines.push(format!("{path}\t{}\t{at}\n", entry.meta.len));
            }
        }
    }
    // Sorted, so the digest does not depend on the order a server lists in.
    lines.sort();
    for line in &lines {
        hasher.update(line.as_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}
