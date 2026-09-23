//! A governed folder that keeps itself in order.
//!
//! The deciding belongs to [`tungstate_core`] and the moving to
//! [`tungstate_execute`]. This crate only answers *when*, and the answer is
//! made of two parts that are easy to get wrong and easy to test separately:
//!
//! - **When has a file stopped changing.** It already had an answer:
//!   `[defaults].cooldown` has been in the policy since slice 5, and the
//!   planner already holds back anything written more recently than that. So
//!   nothing here implements settling. It comes back later and asks again.
//! - **When to look at all.** An event says a folder is worth surveying. An
//!   hourly sweep says so regardless, because **watchers drop events** and a
//!   watcher that is believed is a folder that silently stops being tidied.
//!
//! The watcher is an optimisation. The sweep is the truth.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use notify::RecursiveMode;
use notify_debouncer_full::new_debouncer;
use tungstate_backend::local::LocalBackend;
use tungstate_core::policy::{Mode, Policy};
use tungstate_journal::Journal;

mod schedule;
#[cfg(test)]
mod tests;

pub use schedule::{Echoes, Schedule, is_ours, root_of};

/// Where a folder's rules live, relative to its root.
const POLICY_RELATIVE: &str = ".tungstate/policy.toml";

/// How long a burst of events is gathered before any of it is looked at.
///
/// Short: this only pairs a rename's two halves and coalesces the several
/// events one save produces. The real waiting is the policy's cooldown.
const DEBOUNCE: Duration = Duration::from_secs(1);

/// How often the loop wakes regardless, so a stop is answered promptly.
const TICK: Duration = Duration::from_millis(250);

/// How long tungstate's own writes are expected back.
///
/// Generous: an event can take a moment to arrive, and expecting one that
/// never comes costs a map entry until it ages out.
const ECHO_FOR: Duration = Duration::from_secs(10);

/// A folder to keep an eye on.
#[derive(Debug, Clone)]
pub struct Watched {
    /// What it is called on screen.
    pub name: String,
    /// Where it is.
    pub root: PathBuf,
    /// Whether the bytes are a network away.
    ///
    /// A share is **not watched**: `FSEvents` and inotify say nothing about a
    /// change another machine made. It gets the sweep and nothing else, and
    /// the difference is said out loud rather than left to be discovered.
    pub networked: bool,
}

/// Something the watcher did or saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Noticed {
    /// Watching has begun.
    Started {
        /// Folders being watched for events.
        watching: usize,
        /// Folders that only get the sweep, because they are on a share.
        sweeping: usize,
    },
    /// Files settled in a folder that files things on its own, and were filed.
    Tidied {
        /// The folder's name.
        folder: String,
        /// Files moved. **Not operations**: a plan also makes and removes
        /// directories, and slice 7b shipped "moved 9 file(s)" for five.
        files: usize,
        /// The plan, so it can be undone.
        plan: i64,
    },
    /// Files settled in a folder that does not move things on its own.
    Waiting {
        /// The folder's name.
        folder: String,
        /// How many files its rules would move.
        files: usize,
    },
    /// A folder was looked at and there was nothing to do.
    Settled {
        /// The folder's name.
        folder: String,
    },
    /// A folder could not be looked at.
    Trouble {
        /// The folder's name.
        folder: String,
        /// What went wrong, as a sentence.
        why: String,
    },
}

/// Anything that stops watching before it starts.
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    /// The platform's watcher could not be set up at all.
    #[error("cannot watch: {0}")]
    Setup(String),
    /// Nothing to watch.
    #[error("no governed folders to watch")]
    Nothing,
}

/// Asked to stop, from another thread.
#[derive(Debug, Clone, Default)]
pub struct Stop(Arc<AtomicBool>);

impl Stop {
    /// A fresh flag, not yet asked.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the loop to end. It finishes what it is doing first.
    pub fn ask(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether stopping has been asked for.
    #[must_use]
    pub fn asked(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// What a single look at a folder is told to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Because {
    /// Something happened in it.
    Stirred,
    /// The clock came round.
    Swept,
}

/// Watch until told to stop.
///
/// `told` hears everything: the command line prints it, the window emits it,
/// a test counts it. Blocking, and owns one thread for the length of the call.
///
/// # Errors
/// [`WatchError`] if the platform's watcher cannot be set up, or if there is
/// nothing to watch.
pub fn watch(
    folders: &[Watched],
    journal: &Journal,
    sweep_every: Duration,
    stop: &Stop,
    told: &mut dyn FnMut(&Noticed),
) -> Result<(), WatchError> {
    if folders.is_empty() {
        return Err(WatchError::Nothing);
    }
    let (sender, events) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(DEBOUNCE, None, sender)
        .map_err(|error| WatchError::Setup(error.to_string()))?;

    let mut watching = 0;
    for folder in folders.iter().filter(|folder| !folder.networked) {
        match debouncer.watch(&folder.root, RecursiveMode::Recursive) {
            Ok(()) => watching += 1,
            // A folder that cannot be watched is still swept. On Linux this is
            // where `max_user_watches` runs out, and the right answer is to
            // say so and carry on rather than to abandon every other folder.
            Err(error) => told(&Noticed::Trouble {
                folder: folder.name.clone(),
                why: format!("cannot watch it, so it will only be checked on the hour: {error}"),
            }),
        }
    }
    told(&Noticed::Started {
        watching,
        sweeping: folders.len() - watching,
    });

    // The path an event carries is the one the platform resolved, not the one
    // we asked about: on macOS a folder under `/tmp` is reported under
    // `/private/tmp`, so a root spelled the first way matches nothing. The
    // journal hit this in slice 8b and it is the same trap.
    let resolved: Vec<(String, &Watched)> = folders
        .iter()
        .map(|folder| (settled_name(&folder.root), folder))
        .collect();
    let roots: Vec<String> = resolved.iter().map(|(root, _)| root.clone()).collect();
    let mut schedule = Schedule::default();
    let mut echoes = Echoes::default();
    let mut next_sweep = Instant::now() + sweep_every;

    while !stop.asked() {
        let now = Instant::now();
        let wait = schedule
            .quiet_for(now)
            .unwrap_or(sweep_every)
            .min(next_sweep.saturating_duration_since(now))
            .min(TICK);

        if let Ok(batch) = events.recv_timeout(wait) {
            match batch {
                Ok(seen) => {
                    let now = Instant::now();
                    for event in seen {
                        for path in &event.paths {
                            note(path, &roots, &resolved, &echoes, now, &mut schedule);
                        }
                    }
                }
                // The platform gave up on some events. The sweep is what
                // covers that, which is the whole reason it exists.
                Err(errors) => {
                    for error in errors {
                        tracing::debug!(%error, "watcher complained");
                    }
                }
            }
        }

        let now = Instant::now();
        echoes.forget_old(now);
        for root in schedule.ready(now) {
            if let Some((_, folder)) = resolved.iter().find(|(at, _)| *at == root) {
                look(folder, journal, Because::Stirred, &mut echoes, told);
            }
        }
        if now >= next_sweep {
            next_sweep = now + sweep_every;
            for folder in folders {
                look(folder, journal, Because::Swept, &mut echoes, told);
            }
        }
    }
    Ok(())
}

/// Sweep every folder once and return. What `--once` and the window's opening
/// pass both want.
///
/// # Errors
/// Never: a folder that cannot be read is reported through `told`.
pub fn sweep(folders: &[Watched], journal: &Journal, told: &mut dyn FnMut(&Noticed)) {
    let mut echoes = Echoes::default();
    for folder in folders {
        look(folder, journal, Because::Swept, &mut echoes, told);
    }
}

/// One event, turned into a deadline or dropped.
fn note(
    path: &Path,
    roots: &[String],
    resolved: &[(String, &Watched)],
    echoes: &Echoes,
    now: Instant,
    schedule: &mut Schedule,
) {
    let Some(root) = root_of(path, roots) else {
        return;
    };
    if is_ours(path, root) || echoes.ours(path, now) {
        return;
    }
    // The cooldown is the folder's own, so two folders with different ideas of
    // "settled" keep them.
    let cooldown = resolved
        .iter()
        .find(|(at, _)| at == root)
        .and_then(|(_, folder)| rules_at(&folder.root).ok())
        .map_or(Duration::from_secs(30), |policy| policy.defaults.cooldown);
    schedule.stirred(root, now, cooldown);
}

/// A root spelled the way the platform will spell it back.
fn settled_name(root: &Path) -> String {
    std::fs::canonicalize(root)
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .to_string()
}

/// Look at one folder, and do whatever its own mode says to do.
fn look(
    folder: &Watched,
    journal: &Journal,
    because: Because,
    echoes: &mut Echoes,
    told: &mut dyn FnMut(&Noticed),
) {
    let policy = match rules_at(&folder.root) {
        Ok(policy) => policy,
        Err(why) => {
            told(&Noticed::Trouble {
                folder: folder.name.clone(),
                why,
            });
            return;
        }
    };
    let backend = LocalBackend::new(folder.root.clone());
    let snapshot = match tungstate_attrs::survey(&backend, &policy) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            told(&Noticed::Trouble {
                folder: folder.name.clone(),
                why: error.to_string(),
            });
            return;
        }
    };
    let plan = policy.plan(&snapshot);
    let files = plan.blast.files;
    if files == 0 {
        // A sweep that found nothing is worth one line; an event that led
        // nowhere is not worth waking anybody for.
        if because == Because::Swept {
            told(&Noticed::Settled {
                folder: folder.name.clone(),
            });
        }
        return;
    }

    // **The one place this slice can move a file.** Everything else reports.
    if policy.folder.mode != Mode::Enforce {
        told(&Noticed::Waiting {
            folder: folder.name.clone(),
            files,
        });
        return;
    }

    let root = folder.root.to_string_lossy().to_string();
    match tungstate_execute::apply(&plan, &snapshot, &backend, journal, &root) {
        Ok(applied) => {
            // Every path it wrote comes back as an event about a file that just
            // changed. Expecting them is what stops each action costing a
            // second survey of the whole folder.
            // Both ends of every move: the source vanished and the target
            // appeared, and the platform reports both.
            let here = PathBuf::from(settled_name(&folder.root));
            echoes.expect(
                plan.ops
                    .iter()
                    .flat_map(|op| [op.source(), op.target(), op.directory()])
                    .flatten()
                    .map(|path| here.join(path).to_string_lossy().to_string()),
                Instant::now() + ECHO_FOR,
            );
            told(&Noticed::Tidied {
                folder: folder.name.clone(),
                files,
                plan: applied.plan.0,
            });
        }
        Err(error) => told(&Noticed::Trouble {
            folder: folder.name.clone(),
            why: error.to_string(),
        }),
    }
}

/// A folder's rules, read from the folder.
///
/// # Errors
/// A sentence: the file is not there, or it will not parse.
pub fn rules_at(root: &Path) -> Result<Policy, String> {
    let path = root.join(POLICY_RELATIVE);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Policy::parse(&text)
        .map(|loaded| loaded.policy)
        .map_err(|error| error.to_string())
}
