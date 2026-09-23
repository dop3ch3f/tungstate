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

use std::collections::BTreeSet;
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

    // Every spelling an event might use for each root, all naming one folder.
    let resolved: Vec<(String, &Watched)> = folders
        .iter()
        .flat_map(|folder| {
            spellings(&folder.root)
                .into_iter()
                .map(move |at| (at, folder))
        })
        .collect();
    let roots: Vec<String> = resolved.iter().map(|(root, _)| root.clone()).collect();
    let mut schedule = Schedule::default();
    let mut echoes = Echoes::default();
    let mut next_sweep = Instant::now() + sweep_every;

    // Look once before listening for anything. Files that arrived while this
    // was not running generated no event anybody heard, so without this the
    // folder is not actually in order until the next sweep, and turning the
    // switch on appears to do nothing at all.
    for folder in folders {
        look(folder, journal, Because::Swept, &mut echoes, told);
    }

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
                        // FSEvents coalesced more than it could report and
                        // says only "look again under here". Events were lost,
                        // so nothing narrower than the whole folder will do.
                        if event.need_rescan() {
                            stir_all(&event.paths, &resolved, now, &mut schedule);
                            continue;
                        }
                        for path in &event.paths {
                            note(path, &roots, &resolved, &echoes, now, &mut schedule);
                        }
                    }
                }
                // The platform gave up on some events. Waiting for the hourly
                // sweep would be correct and an hour late, so look now.
                Err(errors) => {
                    let now = Instant::now();
                    for error in errors {
                        tracing::debug!(%error, "watcher complained");
                        stir_all(&error.paths, &resolved, now, &mut schedule);
                    }
                }
            }
        }

        let now = Instant::now();
        echoes.forget_old(now);
        for key in schedule.ready(now) {
            if let Some(folder) = folders.iter().find(|folder| key_of(folder) == key) {
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
    // An event about the root itself only says its listing changed, which the
    // child's own event already said. Heeding it is a loop: every look probes
    // the filesystem inside the root, which changes the root.
    if path == Path::new(root) || is_ours(path, root) || echoes.ours(path, now) {
        return;
    }
    if let Some((_, folder)) = resolved.iter().find(|(at, _)| at == root) {
        schedule.stirred(&key_of(folder), now, cooldown(folder));
    }
}

/// Events were lost under `paths`, or anywhere if there are none: stir every
/// folder that could have been touched, as if something happened in each.
fn stir_all(
    paths: &[PathBuf],
    resolved: &[(String, &Watched)],
    now: Instant,
    schedule: &mut Schedule,
) {
    for (root, folder) in resolved {
        let touched = paths.is_empty()
            || paths
                .iter()
                .any(|path| path.starts_with(root) || Path::new(root).starts_with(path));
        if touched {
            schedule.stirred(&key_of(folder), now, cooldown(folder));
        }
    }
}

/// How long this folder waits for a file to stop changing. The folder's own,
/// so two folders with different ideas of "settled" keep them.
fn cooldown(folder: &Watched) -> Duration {
    rules_at(&folder.root).map_or(Duration::from_secs(30), |policy| policy.defaults.cooldown)
}

/// One name per folder for the schedule, whichever spelling stirred it.
fn key_of(folder: &Watched) -> String {
    folder.root.to_string_lossy().to_string()
}

/// Every way the platform might spell this root back in an event.
///
/// Platforms disagree about which. `FSEvents` reports the resolved path, so a
/// folder under `/tmp` arrives as `/private/tmp`. Windows reports the path as
/// it was watched, so `C:\Users\RUNNER~1` stays short while resolving makes it
/// long. Resolving goes through the journal's `resolve_for_lookup`, because
/// bare `canonicalize` on Windows adds a `\\?\` prefix no event carries.
fn spellings(root: &Path) -> Vec<String> {
    let given = root.to_string_lossy().to_string();
    let resolved = tungstate_journal::resolve_for_lookup(root)
        .to_string_lossy()
        .to_string();
    if resolved == given {
        vec![given]
    } else {
        vec![given, resolved]
    }
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
            // What actually moved, not what was planned. A plan that was half
            // refused would otherwise be reported as a folder tidied, which is
            // the worst kind of wrong: confident and unattended. A failure
            // only costs a file if it was a file's op: a refused `MkDir` is
            // reported by path too, and moved nothing to begin with.
            let sources: BTreeSet<&str> = plan.ops.iter().filter_map(|op| op.source()).collect();
            let unmoved = applied.skipped.len()
                + applied
                    .failed
                    .iter()
                    .filter(|one| sources.contains(one.path.as_str()))
                    .count();
            let moved = files.saturating_sub(unmoved);
            if !applied.failed.is_empty() {
                told(&Noticed::Trouble {
                    folder: folder.name.clone(),
                    why: format!(
                        "{} file(s) could not be moved: {}",
                        applied.failed.len(),
                        applied
                            .failed
                            .iter()
                            .map(|one| one.why.clone())
                            .collect::<Vec<_>>()
                            .join("; ")
                    ),
                });
            }
            // Every path it wrote comes back as an event about a file that just
            // changed. Expecting them is what stops each action costing a
            // second survey of the whole folder. Both ends of every move, in
            // every spelling the platform might use: the source vanished and
            // the target appeared, and the platform reports both.
            let written: Vec<&str> = plan
                .ops
                .iter()
                .flat_map(|op| [op.source(), op.target(), op.directory()])
                .flatten()
                .collect();
            echoes.expect(
                spellings(&folder.root).iter().flat_map(|here| {
                    written
                        .iter()
                        .map(move |path| Path::new(here).join(path).to_string_lossy().to_string())
                }),
                Instant::now() + ECHO_FOR,
            );
            if moved > 0 {
                told(&Noticed::Tidied {
                    folder: folder.name.clone(),
                    files: moved,
                    plan: applied.plan.0,
                });
            }
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
