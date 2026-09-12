//! The durable drain: move files between backends, verifying each one before
//! touching the original, and survive being interrupted at any point.
//!
//! The ordering that makes this safe is fixed and deliberate:
//!
//! 1. Journal the intent, so an interrupted transfer is identifiable.
//! 2. Stream to a temporary name at the destination, hashing as it goes.
//! 3. Verify.
//! 4. Rename the temporary file into place.
//! 5. Journal the outcome.
//! 6. Only then apply the source policy.
//!
//! Every step can be interrupted. The worst outcome of a crash is a file copied
//! twice, never a file lost, because the source is not touched until the
//! destination is both verified and journaled.

mod conflict;
mod governor;
mod stop;
mod walk;

pub use conflict::{
    Conflict, ConflictResolver, Decision, FixedResolver, Identical, IdenticalAction,
    InteractiveResolver,
};
pub use governor::{Governor, Limits};
pub use stop::{Halt, Stop};

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use tungstate_backend::{Backend, BackendError};
use tungstate_journal::{
    ConflictAction, Journal, JournalError, Link, Location, NewOp, OpKind, Outcome, SourcePolicy,
    VerifyLevel, temp_name,
};

/// Where quarantined files land, relative to the destination root.
const QUARANTINE_DIR: &str = ".tungstate-quarantine";

/// Bytes moved per read. Large enough that syscall overhead disappears against
/// a network round trip, small enough to stay out of the way in memory.
const CHUNK: usize = 1024 * 1024;

/// How often a file in flight reports its progress.
///
/// Four times a second: fast enough that a bar looks alive, slow enough that
/// a long file costs a few hundred events rather than a few thousand.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Anything that can stop a transfer.
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    /// A backend operation failed.
    #[error("storage error")]
    Backend(#[from] BackendError),

    /// The journal could not be read or written.
    #[error("journal error")]
    Journal(#[from] JournalError),

    /// Reading or writing the stream itself failed.
    #[error("transfer of `{path}` failed")]
    Io {
        /// The file being transferred.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },

    /// The destination went away, or became different storage, mid-run.
    #[error("the destination is no longer reachable; nothing further was moved")]
    DestinationLost,

    /// The copy completed but did not match the source.
    #[error("verification failed for `{path}`: {detail}")]
    Verification {
        /// The file that failed to verify.
        path: PathBuf,
        /// What did not match.
        detail: String,
    },

    /// The destination cannot rename, so `replace` cannot keep its promise.
    #[error(
        "`{path}`: this destination cannot rename, so the file already there \
         cannot be moved aside; use --on-conflict quarantine, rename or skip"
    )]
    ReplaceNeedsRename {
        /// The file whose conflict could not be resolved that way.
        path: PathBuf,
    },

    /// A hard stop landed mid-file, and this file was put down where it was.
    ///
    /// Its own variant rather than an `Io` error because it is not a failure:
    /// the run was told to do this, and reporting it beside genuine failures
    /// would make a deliberate stop look like a fault.
    #[error("`{path}` was abandoned when the run was stopped")]
    Stopped {
        /// The file that was being written when the stop arrived.
        path: PathBuf,
    },

    /// The source is remote, and there is no trash to send an original to.
    #[error("`{path}` is on a remote, which has no trash; use --move or --copy")]
    TrashUnsupported {
        /// The file whose original could not be trashed.
        path: PathBuf,
    },

    /// The original could not be sent to the operating system's trash.
    #[error("could not trash `{path}`")]
    Trash {
        /// The file that could not be trashed.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: trash::Error,
    },
}

/// Result alias so signatures read `Result<Summary>`.
pub type Result<T> = std::result::Result<T, TransferError>;

/// What happened to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOutcome {
    /// Copied, verified, and the source policy applied.
    Transferred,
    /// Already present at the destination with identical content, having read
    /// both copies to be sure. Carries what became of the original here.
    AlreadyPresent(Original),
    /// Left alone, with the reason.
    Skipped(SkipReason),
    /// Parked under the quarantine directory.
    Quarantined,
    /// Could not be transferred. The original was left untouched.
    Failed,
}

/// What happened to the copy on this machine when the destination already
/// matched it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Original {
    /// The source policy was applied: deleted, or trashed.
    Removed,
    /// Left here, either because this was a copy or because it was asked.
    Kept,
}

/// One file that could not be transferred, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Path relative to the source root.
    pub path: PathBuf,
    /// Human-readable reason, including the underlying cause.
    pub reason: String,
}

/// Why a file was left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// Written too recently to be safe to copy; it may still be growing.
    RecentlyModified,
    /// Something else holds the name and the resolver chose to leave it.
    Conflict,
}

/// Totals for one run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// Files copied and verified.
    pub transferred: u64,
    /// Files already at the destination with identical content.
    pub already_present: u64,
    /// Files deliberately left alone.
    pub skipped: u64,
    /// Files parked in quarantine for the user to resolve.
    pub quarantined: u64,
    /// Bytes successfully transferred.
    pub bytes: u64,
    /// Interrupted operations cleaned up before this run started.
    pub recovered: u64,
    /// Source directories removed because the drain emptied them.
    pub pruned: u64,
    /// Files that could not be transferred. Their originals are untouched.
    pub failed: u64,
    /// Detail for each failure, for reporting at the end of a run.
    pub failures: Vec<Failure>,
    /// The run stopped early because it was asked to.
    pub cancelled: bool,
    /// The run stopped because the destination stopped being the destination.
    pub destination_lost: bool,
}

/// One file the run intends to deal with, in the order it will be taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// Path relative to the source root.
    pub path: PathBuf,
    /// Size in bytes.
    pub size: u64,
}

/// What a run turned out to be, once the far side has been asked.
///
/// Both facts are decided inside the engine and neither is guessable from
/// outside: a caller knows which button was pressed, not what the destination
/// agreed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunShape {
    /// Whether the originals will be removed. A copy reported as "freed from
    /// this machine" reads as data loss to someone watching it happen.
    pub removes_originals: bool,
    /// How many files will be in flight, after the handshake settled.
    pub at_once: usize,
}

/// Told about each file as it is dealt with, so a caller can show progress.
///
/// `Send` because a run reports from its worker threads. As with
/// [`ConflictResolver`], the run holds one behind a lock, so lines never
/// interleave and an implementation does not have to be thread-safe itself.
pub trait Progress: Send {
    /// What this run is, before any of it happens.
    ///
    /// Separate from `planned` because it answers a different question — not
    /// *which* files, but what will become of them and how fast. Emitted once
    /// per link, so an exchange reports twice.
    fn began(&mut self, _shape: RunShape) {}

    /// The run has narrowed or widened since it began.
    ///
    /// Separate from `began` because `began` is a promise about one moment
    /// and this is a correction to it: a width chosen by handshake can turn
    /// out to be more than the far side can usefully do, and the window
    /// showing "4 at a time" while two are parked would be another comfortable
    /// lie of the kind this slice exists to remove.
    fn at_once(&mut self, _files: usize) {}

    /// The whole plan, once, before the first file.
    ///
    /// Defaulted because not every caller wants it, and because this arrived
    /// after the trait had implementations. The moment matters: it is the only
    /// point at which the full list and its order exist, so it is the only
    /// chance to show what is *next* rather than what has already happened.
    fn planned(&mut self, _files: &[Planned]) {}

    /// A file is about to be transferred.
    fn starting(&mut self, path: &Path, size: u64);

    /// Both copies are being read to find out whether they match.
    ///
    /// Its own phase because it is not copying and moves no bytes anywhere: a
    /// row that says "copying, 0 bytes" through two full-file reads — one of
    /// them across the network — looks exactly like a stalled transfer.
    /// `total` spans both reads, so the bar crosses the whole check once
    /// rather than filling twice.
    fn checking(&mut self, _path: &Path, _done: u64, _total: u64) {}

    /// Bytes have moved for the file in flight.
    ///
    /// Throttled by the engine, not by the caller: a 4 GB file passes through
    /// the streaming loop four thousand times, and the thing that knows how
    /// often that is worth mentioning is the thing doing the work. Always
    /// emitted once with `done == total`, so a bar can finish exactly.
    fn advanced(&mut self, _path: &Path, _done: u64, _total: u64) {}

    /// A file has been dealt with.
    fn finished(&mut self, path: &Path, outcome: FileOutcome);
}

/// A `Progress` that discards everything, for tests and quiet runs.
#[derive(Debug, Default)]
pub struct SilentProgress;

impl Progress for SilentProgress {
    fn starting(&mut self, _path: &Path, _size: u64) {}
    fn finished(&mut self, _path: &Path, _outcome: FileOutcome) {}
}

/// What would happen to one file, without anything happening to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prospect {
    /// Nothing holds that name; it would be copied across.
    Fresh,
    /// Something of the same size is already there. Whether it is the same file
    /// is only knowable by reading both, which a preview deliberately does not
    /// do; the run itself compares fingerprints and decides.
    SameSize {
        /// Size of the file already at the destination.
        existing: u64,
    },
    /// Something of a different size holds that name, so it is a genuine clash.
    Clash {
        /// Size of the file already at the destination.
        existing: u64,
    },
    /// Written too recently to be safe to move; it would be left for next time.
    TooRecent,
}

/// One line of a preview.
#[derive(Debug, Clone)]
pub struct Prospective {
    /// Path relative to the source root.
    pub path: PathBuf,
    /// Where it would land, relative to the destination root.
    pub destination: PathBuf,
    /// Size in bytes.
    pub size: u64,
    /// What would happen.
    pub prospect: Prospect,
}

/// Everything a run would do, computed without touching either side.
#[derive(Debug, Clone, Default)]
pub struct Preview {
    /// Every file the run would consider, largest first.
    pub items: Vec<Prospective>,
    /// Files that would be copied across.
    pub fresh: u64,
    /// Files whose name is taken by something the same size.
    pub same_size: u64,
    /// Files whose name is taken by something different.
    pub clashes: u64,
    /// Files that would be left for next time.
    pub too_recent: u64,
    /// Bytes that would move, excluding anything held back.
    pub bytes: u64,
    /// True when the originals would be removed once verified.
    pub removes_originals: bool,
}

/// Work out what a run would do, changing nothing.
///
/// Deliberately cheap: it compares sizes rather than fingerprints, because
/// hashing both sides would read every byte twice before the user has agreed to
/// anything. A same-size pair is reported as such rather than guessed at.
///
/// # Errors
/// [`TransferError`] if either side cannot be listed, or if the destination is
/// not reachable, which is worth knowing before agreeing to a transfer.
pub fn preview(
    link: &Link,
    source: &dyn Backend,
    destination: &dyn Backend,
    only: Option<&[PathBuf]>,
) -> Result<Preview> {
    destination.root_token()?;

    let mut files = match only {
        Some(chosen) => gather(source, chosen)?,
        None => walk::files(source)?,
    };
    walk::sort(&mut files, link.order);

    let mut preview = Preview {
        removes_originals: link.source_policy != SourcePolicy::Keep,
        ..Preview::default()
    };

    for file in files {
        let landing = file.path.clone();
        let prospect = if within(link, &file) {
            preview.too_recent += 1;
            Prospect::TooRecent
        } else {
            match destination.stat(&landing) {
                Err(_) => {
                    preview.fresh += 1;
                    preview.bytes += file.size;
                    Prospect::Fresh
                }
                Ok(existing) if existing.len == file.size => {
                    preview.same_size += 1;
                    preview.bytes += file.size;
                    Prospect::SameSize {
                        existing: existing.len,
                    }
                }
                Ok(existing) => {
                    preview.clashes += 1;
                    preview.bytes += file.size;
                    Prospect::Clash {
                        existing: existing.len,
                    }
                }
            }
        };

        preview.items.push(Prospective {
            path: file.path,
            destination: landing,
            size: file.size,
            prospect,
        });
    }

    Ok(preview)
}

/// Expand a chosen set into the files it covers.
///
/// A path that cannot be read is skipped rather than failing the run. On a
/// resumed transfer that is the normal case: the files already moved are gone
/// from the source, and their absence is the drain working, not a fault.
fn gather(source: &dyn Backend, chosen: &[PathBuf]) -> Result<Vec<walk::File>> {
    let mut files = Vec::new();
    for path in chosen {
        let Ok(meta) = source.stat(path) else {
            tracing::debug!(path = %path.display(), "already gone from the source");
            continue;
        };
        if meta.is_dir {
            files.extend(walk::files_under(source, path)?);
        } else if !meta.is_symlink {
            files.push(walk::File {
                path: path.clone(),
                size: meta.len,
                modified: meta.modified,
            });
        }
    }
    Ok(files)
}

fn within(link: &Link, file: &walk::File) -> bool {
    file.modified.is_some_and(|modified| {
        SystemTime::now()
            .duration_since(modified)
            .is_ok_and(|age| age < link.cooldown)
    })
}

/// One run of one link.
pub struct Transfer<'a> {
    link: &'a Link,
    source: &'a dyn Backend,
    destination: &'a dyn Backend,
    journal: &'a Journal,
    // Behind locks so several files can be in flight while a conflict is
    // still asked once at a time and progress lines never interleave. An
    // implementation of either trait does not have to be thread-safe itself;
    // it only has to be `Send`, which is why those bounds exist.
    resolver: Mutex<&'a mut dyn ConflictResolver>,
    progress: Mutex<&'a mut dyn Progress>,
    // Every destination this run will write to: the whole plan, claimed
    // before any worker starts, plus each name a rename has taken since.
    //
    // Probing the backend for a free name is not enough once files are in
    // flight. `holiday.mp4` clashes and is renamed to `holiday-2.mp4` at the
    // same moment another worker is transferring a source file genuinely
    // called `holiday-2.mp4` to exactly that name — both find the slot empty
    // and one silently overwrites the other. Sequentially each always sees
    // the other, so this is a hazard concurrency creates.
    claimed: Mutex<std::collections::BTreeSet<PathBuf>>,
    // Checked between files at either strength, and inside the copy and check
    // loops at the stronger one.
    cancel: Option<Arc<Stop>>,
    /// How long the governor times one width for. Only a test sets this; a
    /// real run uses the governor's own window.
    #[cfg(test)]
    window: Option<Duration>,
    /// Bytes written by every worker this run, for the governor to divide by
    /// a window. Relaxed: it feeds a throughput estimate, and an exact value
    /// would not change any decision made from it.
    moved: AtomicU64,
    /// An explicit ceiling, when the user has one in mind. Raises the limit a
    /// run will climb to; never removes the handshake gate or the back-off.
    parallel: Option<usize>,
}

/// A poisoned lock means another worker panicked. The data behind it is still
/// sound, and refusing every later file would turn one bug into a dead run.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl<'a> Transfer<'a> {
    /// Assemble a run.
    #[must_use]
    pub fn new(
        link: &'a Link,
        source: &'a dyn Backend,
        destination: &'a dyn Backend,
        journal: &'a Journal,
        resolver: &'a mut dyn ConflictResolver,
        progress: &'a mut dyn Progress,
    ) -> Self {
        Self {
            link,
            source,
            destination,
            journal,
            resolver: Mutex::new(resolver),
            progress: Mutex::new(progress),
            claimed: Mutex::new(std::collections::BTreeSet::new()),
            cancel: None,
            #[cfg(test)]
            window: None,
            moved: AtomicU64::new(0),
            parallel: None,
        }
    }

    /// Stop when asked, at whichever strength was asked for.
    ///
    /// [`Halt::AfterThisFile`] is honoured between files, so nothing in flight
    /// is abandoned. [`Halt::Now`] is honoured inside the copy and check
    /// loops, so it may leave a partial file — which recovery already handles,
    /// because it is the same thing a killed process leaves.
    #[must_use]
    pub fn cancellable(mut self, stop: Arc<Stop>) -> Self {
        self.cancel = Some(stop);
        self
    }

    /// Raise the ceiling this run will climb to.
    ///
    /// The floor, the handshake before each promotion and the back-off on
    /// objection all still apply: asking for eight does not make a server
    /// give eight, it only means tungstate will keep asking until it is told
    /// no.
    #[must_use]
    pub fn parallel(mut self, files_at_once: usize) -> Self {
        self.parallel = Some(files_at_once);
        self
    }

    /// One worker: take a file, deal with it, fold the result in, repeat.
    ///
    /// `index` is how a run shrinks. Threads cannot be un-spawned, so a worker
    /// whose index has fallen outside the limit finishes what it holds and
    /// leaves rather than taking another file.
    fn work(
        &self,
        index: usize,
        queue: &Mutex<std::collections::VecDeque<walk::File>>,
        shared: &Mutex<Summary>,
        governor: &Governor,
        anchor: &tungstate_backend::RootToken,
    ) {
        loop {
            if self.cancelled() {
                tracing::info!("stopping at the user's request");
                lock(shared).cancelled = true;
                return;
            }

            // Between files is the only point a width change can take effect:
            // a worker part-way through a copy cannot stand down without
            // throwing the work away.
            if let Some(width) = governor.reconsider(self.moved.load(Ordering::Relaxed)) {
                lock(&self.progress).at_once(width);
            }

            // Park rather than return when the run has narrowed. A returned
            // thread cannot be recalled, which would make the limit one-way
            // and a measurement that guessed wrong permanent.
            let empty = || lock(queue).is_empty() || self.cancelled();
            if !governor.wait_for_room(index, &empty) {
                tracing::debug!(index, "standing down; nothing left to take");
                return;
            }

            // Checked per file rather than once: a NAS can drop out at any
            // point, and every file after that would otherwise be written to
            // whatever now sits at that path, then have its original deleted.
            match self.destination.root_token() {
                Ok(token) if token == *anchor => {}
                _ => {
                    tracing::error!("destination is no longer the storage we started with");
                    lock(shared).destination_lost = true;
                    return;
                }
            }

            let Some(file) = lock(queue).pop_front() else {
                return;
            };

            // One unreadable file must not abandon the other three thousand.
            // The failure is journaled, reported, and the drain carries on;
            // `failures` is what the caller shows at the end.
            let outcome = match self.transfer_one(&file) {
                Ok(outcome) => outcome,
                // Asked for, so not reported beside genuine failures. The file
                // stays where it was in every sense: original untouched, op
                // still open, partial findable.
                Err(TransferError::Stopped { .. }) => {
                    lock(shared).cancelled = true;
                    return;
                }
                Err(error) => {
                    // The far side saying "not so many at once" is not this
                    // file's fault, and the answer is fewer workers rather
                    // than fewer files.
                    if governor::is_overload(&error) {
                        governor.rebuff();
                    }
                    tracing::warn!(path = %file.path.display(), %error, "transfer failed");
                    let mut summary = lock(shared);
                    summary.failed += 1;
                    summary.failures.push(Failure {
                        path: file.path.clone(),
                        reason: explain(&error),
                    });
                    FileOutcome::Failed
                }
            };
            lock(&self.progress).finished(&file.path, outcome);

            let mut summary = lock(shared);
            match outcome {
                FileOutcome::Transferred => {
                    summary.transferred += 1;
                    summary.bytes += file.size;
                }
                FileOutcome::AlreadyPresent(_) => summary.already_present += 1,
                FileOutcome::Skipped(_) => summary.skipped += 1,
                FileOutcome::Quarantined => summary.quarantined += 1,
                // Already counted above; the source was left untouched.
                FileOutcome::Failed => {}
            }
        }
    }

    /// Whether the run should stop before picking up another file.
    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|stop| stop.asked())
    }

    /// Whether the run should drop what it is holding, mid-file.
    fn abandoning(&self) -> bool {
        self.cancel.as_ref().is_some_and(|stop| stop.immediate())
    }

    /// Transfer everything the link covers.
    ///
    /// Resumes first: any operation this link left unfinished has its partial
    /// file removed and is re-queued, which is safe because a re-queued file
    /// whose destination already matches is recognised as already present.
    ///
    /// # Errors
    /// [`TransferError`] if the source cannot be walked, or if a file fails in a
    /// way that should stop the run. Per-file conflicts are resolved rather than
    /// returned.
    pub fn run(&mut self) -> Result<Summary> {
        // The link's own record of what it was asked to move. Consulted here
        // rather than passed in by every caller, because "remember to hand
        // the right selection to the right run" is exactly what went wrong:
        // a resumed browser transfer was handed nothing and walked the whole
        // source root, past everything the user had actually chosen.
        let chosen = self.journal.files_for(self.link.id)?;
        let files = if chosen.is_empty() {
            walk::files(self.source)?
        } else {
            gather(self.source, &chosen)?
        };
        self.carry(files)
    }

    /// Transfer only the named paths, relative to the source root.
    ///
    /// A directory in the list brings everything under it. This is what the
    /// browser's Move and Copy use, where the user has chosen the files rather
    /// than asking for the whole folder.
    ///
    /// # Errors
    /// As [`Transfer::run`].
    pub fn run_selection(&mut self, chosen: &[PathBuf]) -> Result<Summary> {
        let files = gather(self.source, chosen)?;
        self.carry(files)
    }

    /// How many transfers this destination can take at once.
    ///
    /// A property of the place, not a setting: a mounted volume has no
    /// per-client connection limit to exceed and a protocol does.
    fn limits(&self) -> Limits {
        let networked = self
            .link
            .destination
            .connection
            .and_then(|id| self.journal.connection_by_id(id).ok())
            .is_some_and(|connection| connection.scheme.is_networked());

        let limits = if networked {
            Limits::networked()
        } else {
            Limits::local()
        };
        match self.parallel {
            Some(requested) => limits.overridden(requested),
            None => limits,
        }
    }

    /// Make the governor decide on a short window, so a test can watch the
    /// width move without sleeping through several real ones.
    #[cfg(test)]
    pub(crate) fn sampling_every(mut self, window: Duration) -> Self {
        self.window = Some(window);
        self
    }

    #[cfg(test)]
    fn new_governor(&self) -> Governor {
        match self.window {
            Some(window) => Governor::sampling_every(self.limits(), window),
            None => Governor::new(self.limits()),
        }
    }

    #[cfg(not(test))]
    fn new_governor(&self) -> Governor {
        Governor::new(self.limits())
    }

    fn carry(&mut self, mut files: Vec<walk::File>) -> Result<Summary> {
        // Pin what the destination is before anything moves, so a volume
        // swapped underneath us is detectable rather than silently written to.
        let anchor = self.destination.root_token()?;

        let recovered = self.recover()?;

        walk::sort(&mut files, self.link.order);

        // Claim every destination the plan will occupy, so a rename cannot
        // choose a name that a file still waiting is going to need.
        {
            let mut claimed = lock(&self.claimed);
            for file in &files {
                claimed.insert(file.path.clone());
            }
        }

        // Find the limit before the first file, by handshake. Climbing during
        // the run would mean a real transfer is the thing that discovers the
        // ceiling; asking first means a refusal costs a rejected connection.
        let governor = self.new_governor();
        loop {
            let before = governor.limit();
            if governor.try_promote(self.destination) == before {
                break;
            }
        }
        governor.agree();
        let workers = governor.agreed().min(files.len().max(1));
        tracing::info!(workers, files = files.len(), "starting");

        // Ahead of the plan so the caller can word the plan correctly. The
        // handshake above costs one stat, which is why this is not first.
        lock(&self.progress).began(RunShape {
            removes_originals: self.link.source_policy != SourcePolicy::Keep,
            at_once: workers,
        });

        // Everything, in the order it will happen, before anything happens.
        lock(&self.progress).planned(
            &files
                .iter()
                .map(|f| Planned {
                    path: f.path.clone(),
                    size: f.size,
                })
                .collect::<Vec<_>>(),
        );

        let queue = Mutex::new(std::collections::VecDeque::from(files));
        let shared = Mutex::new(Summary {
            recovered,
            ..Summary::default()
        });

        // A shared reborrow: every worker needs `&Transfer`, and `carry` holds
        // `&mut`. This is what the locks on `resolver` and `progress` bought —
        // the per-file work needs nothing mutable from here.
        let me: &Self = self;
        std::thread::scope(|scope| {
            for index in 0..workers {
                let (queue, shared, governor, anchor) = (&queue, &shared, &governor, &anchor);
                scope.spawn(move || me.work(index, queue, shared, governor, anchor));
            }
        });

        let mut summary = lock(&shared).clone();

        // Pruning a half-drained tree would remove directories the remaining
        // files still need, so a cancelled run leaves the structure alone.
        // Only prune when we were the ones emptying directories. A --copy run
        // has removed nothing, so anything empty was already empty.
        if self.link.source_policy != SourcePolicy::Keep
            && !summary.cancelled
            && !summary.destination_lost
        {
            summary.pruned = walk::prune_empty(self.source, Path::new(""))?;
        }

        Ok(summary)
    }

    /// Clean up after an interrupted run, returning how many operations were found.
    ///
    /// A partial file is deleted rather than resumed: slice 3 resumes per file,
    /// not per byte, and a partial whose length cannot be trusted is worse than
    /// no partial at all.
    fn recover(&self) -> Result<u64> {
        let interrupted = self.journal.incomplete_for_link(self.link.id)?;

        for op in &interrupted {
            clear_partial(self.destination, op);
            self.journal.finish(
                op.id,
                &Outcome::Failed {
                    error: "interrupted; re-queued".to_string(),
                },
            )?;
        }

        Ok(interrupted.len() as u64)
    }

    /// Delete temporary files a backend left beside `destination` when it died.
    ///
    /// Only for backends that cannot rename, because those are the ones that
    /// may be doing their own temp-and-rename internally. `OpenDAL`'s FTP
    /// service is the case in hand: it streams to `<name>.<8 random chars>`
    /// and renames on close, and because the name is random neither it nor we
    /// can find it again afterwards. Left alone, a NAS accumulates one of
    /// these next to every transfer that was ever interrupted.
    ///
    /// Scoped hard: only the interrupted operation's own directory, only names
    /// that are this destination's name plus the exact suffix shape. Assumes
    /// no second process is mid-write to the same destination path, which is
    /// already true — a link is run one at a time.
    fn transfer_one(&self, file: &walk::File) -> Result<FileOutcome> {
        lock(&self.progress).starting(&file.path, file.size);

        if self.within_cooldown(file) {
            tracing::debug!(path = %file.path.display(), "still being written; leaving for next run");
            return Ok(FileOutcome::Skipped(SkipReason::RecentlyModified));
        }

        // Mirror the source tree. There is no policy engine until slice 5, so
        // inventing a layout here would be a guess we would have to undo.
        let destination = file.path.clone();

        match self.classify(file, &destination)? {
            Placement::Fresh => self
                .copy_to(file, &destination)
                .map(|()| FileOutcome::Transferred),
            Placement::Identical => self
                .record_already_present(file, &destination)
                .map(FileOutcome::AlreadyPresent),
            Placement::Conflicting => self.resolve_conflict(file, &destination),
        }
    }

    /// Whether the file was written too recently to be safe to copy.
    fn within_cooldown(&self, file: &walk::File) -> bool {
        let Some(modified) = file.modified else {
            // A backend that cannot report a time gets the benefit of the doubt;
            // refusing everything would make such a backend unusable.
            return false;
        };
        SystemTime::now()
            .duration_since(modified)
            .is_ok_and(|age| age < self.link.cooldown)
    }

    fn classify(&self, file: &walk::File, destination: &Path) -> Result<Placement> {
        let Ok(existing) = self.destination.stat(destination) else {
            return Ok(Placement::Fresh);
        };

        // Cheap check first: different sizes cannot be the same content.
        if existing.len != file.size {
            return Ok(Placement::Conflicting);
        }

        // Two full reads, one of them possibly across the network, before a
        // single byte can be sent. Reported as its own phase for exactly that
        // reason, and throttled on the same interval a copy uses.
        let total = file.size * 2;
        let mut done = 0_u64;
        let mut reported = Instant::now();
        let mut watching = |read: u64| {
            done += read;
            if reported.elapsed() >= PROGRESS_INTERVAL {
                reported = Instant::now();
                lock(&self.progress).checking(&file.path, done, total);
            }
            !self.abandoning()
        };

        lock(&self.progress).checking(&file.path, 0, total);
        let here = digest_watching(self.source, &file.path, &mut watching)?;
        let there = digest_watching(self.destination, destination, &mut watching)?;
        lock(&self.progress).checking(&file.path, total, total);

        if here == there {
            Ok(Placement::Identical)
        } else {
            Ok(Placement::Conflicting)
        }
    }

    fn resolve_conflict(&self, file: &walk::File, destination: &Path) -> Result<FileOutcome> {
        let existing = self.destination.stat(destination)?;
        let action = lock(&self.resolver).resolve(&Conflict {
            path: file.path.clone(),
            incoming_size: file.size,
            existing_size: existing.len,
        });

        match action {
            ConflictAction::Skip => Ok(FileOutcome::Skipped(SkipReason::Conflict)),

            ConflictAction::Rename => {
                let renamed = self.disambiguate(destination);
                self.copy_to(file, &renamed)?;
                Ok(FileOutcome::Transferred)
            }

            ConflictAction::Quarantine => {
                let parked = Path::new(QUARANTINE_DIR).join(destination);
                self.copy_to(file, &parked)?;
                Ok(FileOutcome::Quarantined)
            }

            ConflictAction::Replace => {
                // Replace promises the existing file is preserved, and the only
                // way to keep that promise is to move it aside first. Without
                // rename there is no way to move anything, and writing the
                // incoming file first would destroy the very thing Replace
                // undertakes to keep. Refuse rather than quietly do the
                // opposite of what was asked.
                if !self.destination.capabilities().atomic_rename {
                    return Err(TransferError::ReplaceNeedsRename {
                        path: file.path.clone(),
                    });
                }
                // The existing file is quarantined rather than deleted. Replace is
                // the user's decision about which copy they want, not permission
                // to destroy the other one.
                let parked = Path::new(QUARANTINE_DIR).join(destination);
                if let Some(parent) = parked.parent() {
                    self.destination.create_dir_all(parent)?;
                }
                self.destination.rename(destination, &parked)?;
                self.copy_to(file, destination)?;
                Ok(FileOutcome::Transferred)
            }
        }
    }

    /// The core sequence. Journal, copy to a temp name, verify, commit, then and
    /// only then deal with the source.
    fn copy_to(&self, file: &walk::File, destination: &Path) -> Result<()> {
        let op = self.journal.begin(&NewOp {
            kind: match self.link.source_policy {
                SourcePolicy::Keep => OpKind::Copy,
                SourcePolicy::Delete | SourcePolicy::Trash => OpKind::Move,
            },
            source: Some(Location::within(&self.link.source, file.path.clone())),
            destination: Some(Location::within(
                &self.link.destination,
                destination.to_path_buf(),
            )),
            size: Some(file.size),
            link: Some(self.link.name.clone()),
            link_id: Some(self.link.id),
        })?;

        let result = self.copy_verify_commit(file, destination, op);

        match &result {
            Ok(hash) => {
                self.journal.finish(
                    op,
                    &Outcome::Committed {
                        hash: Some(hash.clone()),
                    },
                )?;
                self.apply_source_policy(&file.path)?;
            }
            // Left `Intended` on purpose when the run was stopped: that is
            // the state `interrupted()` looks for, and it is what puts the
            // abandoned file in front of the user with Resume and Clean up
            // beside it. Marking it Failed would hide it.
            Err(TransferError::Stopped { .. }) => {}
            Err(error) => {
                self.journal.finish(
                    op,
                    &Outcome::Failed {
                        error: error.to_string(),
                    },
                )?;
            }
        }

        result.map(|_| ())
    }

    fn copy_verify_commit(
        &self,
        file: &walk::File,
        destination: &Path,
        op: tungstate_journal::OpId,
    ) -> Result<String> {
        if let Some(parent) = destination.parent() {
            self.destination.create_dir_all(parent)?;
        }

        // `Capabilities::atomic_rename` has existed since slice 1 and this is
        // the first thing to read it. FTP is why: OpenDAL's FTP service
        // answers `rename` with `Unsupported`, so the temp-then-rename dance
        // below cannot even be attempted there.
        if !self.destination.capabilities().atomic_rename {
            return self.write_in_place(file, destination);
        }

        let temp = temp_name(destination, op);
        let (hash, written) = self.stream(&file.path, &temp)?;

        if let Err(error) = self.verify(file, &temp, &hash, written) {
            // Leave nothing half-written behind for a later run to trip over.
            let _ = self.destination.remove_file(&temp);
            return Err(error);
        }

        // Atomic where the backend supports it, so the destination name never
        // refers to a partial file.
        self.destination.rename(&temp, destination)?;
        Ok(hash)
    }

    /// Publish by writing to the destination name, for a backend that cannot
    /// rename.
    ///
    /// There is no third option: without rename, the only route to the final
    /// name is to write to it. What protects the commit is
    /// [`WriteFinish::finish`], which the trait already defines as "commit
    /// everything written durably, then close" — how a backend achieves that
    /// is its own business. `OpenDAL`'s FTP service, for instance, streams to a
    /// temporary name of its own and renames on close, so the publish is still
    /// atomic even though `rename` is unavailable to us.
    ///
    /// The exposure this adds is the window before `finish`. A rename backend
    /// can only ever leave a `.part`; here the real name may hold a partial,
    /// so another program watching the folder could see an incomplete file.
    /// Recovery deletes it and the source is untouched either way, so nothing
    /// is lost. How wide the window actually is depends on the backend: one
    /// doing its own temp-and-rename, as `OpenDAL`'s FTP service does, never
    /// exposes the real name at all. The engine cannot tell, so it assumes
    /// the worst and recovery cleans up for both.
    fn write_in_place(&self, file: &walk::File, destination: &Path) -> Result<String> {
        let (hash, written) = self.stream(&file.path, destination)?;

        if let Err(error) = self.verify(file, destination, &hash, written) {
            let _ = self.destination.remove_file(destination);
            return Err(error);
        }

        Ok(hash)
    }

    /// Copy bytes, computing the digest from the same read.
    ///
    /// One pass over the source produces both the copy and the hash; reading it
    /// twice would double the cost of the most expensive part of a drain.
    fn stream(&self, source: &Path, destination: &Path) -> Result<(String, u64)> {
        let total = self.source.stat(source).map_or(0, |m| m.len);
        let mut reader = self.source.open_read(source)?;
        let mut writer = self.destination.create_write(destination)?;
        let mut reported = Instant::now();
        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0_u8; CHUNK];
        let mut written = 0_u64;

        loop {
            let read = reader.read(&mut buffer).map_err(|e| TransferError::Io {
                path: source.to_path_buf(),
                source: e,
            })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            writer
                .write_all(&buffer[..read])
                .map_err(|e| TransferError::Io {
                    path: destination.to_path_buf(),
                    source: e,
                })?;
            written += read as u64;
            self.moved.fetch_add(read as u64, Ordering::Relaxed);

            // Checked per chunk, which is what makes "stop now" mean now: a
            // 4 GB file would otherwise hold the run open for minutes after
            // the button was pressed. The partial left behind is the same
            // shape a killed process leaves, and recovery already handles it.
            if self.abandoning() {
                return Err(TransferError::Stopped {
                    path: source.to_path_buf(),
                });
            }

            // Throttled here rather than in the caller. A 4 GB file passes
            // through this loop four thousand times, and a window that redrew
            // four thousand times would spend longer painting than copying.
            if reported.elapsed() >= PROGRESS_INTERVAL {
                reported = Instant::now();
                lock(&self.progress).advanced(source, written, total);
            }
        }

        // Once at the end regardless of the throttle, so a bar lands on full
        // rather than stopping at whatever the last tick happened to catch.
        lock(&self.progress).advanced(source, written, written);

        // finish() consumes the writer, so it cannot be used afterwards, and it
        // fsyncs. Without that the bytes would only be in the page cache and a
        // power cut after the journal commit could leave a deleted original and
        // an empty destination. It also closes the handle, which Windows
        // requires before the rename.
        writer.finish()?;

        Ok((hasher.finalize().to_hex().to_string(), written))
    }

    fn verify(&self, file: &walk::File, temp: &Path, hash: &str, written: u64) -> Result<()> {
        let landed = self.destination.stat(temp)?;

        if landed.len != written {
            return Err(TransferError::Verification {
                path: file.path.clone(),
                detail: format!("wrote {written} bytes, destination holds {}", landed.len),
            });
        }

        match self.link.verify {
            // These two coincide today, and clippy is right to notice. The
            // length check above satisfies `Size`, and `Hash` adds nothing on
            // top because the digest is computed from the bytes actually sent
            // during the single read the copy already needs. Both trust the
            // storage layer not to corrupt what it acknowledged. They separate
            // in slice 12, where a remote backend can offer a server-side
            // checksum that `Hash` will compare against and `Size` will not.
            VerifyLevel::Size | VerifyLevel::Hash => Ok(()),

            // The only level that proves the bytes on the far disk are the bytes
            // sent, at the cost of reading the whole file back.
            VerifyLevel::Readback => {
                let readback = digest(self.destination, temp)?;
                if readback == hash {
                    Ok(())
                } else {
                    Err(TransferError::Verification {
                        path: file.path.clone(),
                        detail: "readback digest differs from what was sent".to_string(),
                    })
                }
            }
        }
    }
}

/// An error and everything underneath it, on one line.
///
/// The top line of a `TransferError` is deliberately short — "transfer of
/// `a.mp4` failed" — and on its own it is useless in the end-of-run report,
/// which is the only place most users will ever see why something failed. The
/// cause is the whole point.
fn explain(error: &dyn std::error::Error) -> String {
    use std::fmt::Write as _;

    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        // Writing into a String cannot fail, and rendering an error must not
        // itself be fallible.
        let _ = write!(message, ": {cause}");
        source = cause.source();
    }
    message
}

/// Remove whatever an interrupted operation left at the destination.
///
/// One function rather than one per caller. Resuming and discarding both have
/// to clear exactly the same things, and a sweep that matched in one path and
/// not the other is how a file gets deleted that should not have been.
///
/// Safe to call for an operation the journal still shows as `intended`,
/// because `copy_to` applies the source policy only after the commit such an
/// operation never reached: the original is still on the source.
fn clear_partial(destination: &dyn Backend, op: &tungstate_journal::Op) {
    let Some(landing) = &op.destination else {
        return;
    };

    if destination.capabilities().atomic_rename {
        let partial = temp_name(&landing.path, op.id);
        // Already gone is the common case and not an error.
        let _ = destination.remove_file(&partial);
        return;
    }

    // Without rename nothing was written to a temp name, so the only partial
    // there can be is under the real name.
    let _ = destination.remove_file(&landing.path);
    sweep_orphans(destination, &landing.path);
}

/// Delete temporary files a backend left beside `destination` when it died.
///
/// Only for backends that cannot rename, because those are the ones that may
/// be doing their own temp-and-rename internally. `OpenDAL`'s FTP service is the
/// case in hand: it streams to `<name>.<8 random chars>` and renames on close,
/// and because the name is random neither it nor we can find it again
/// afterwards. Left alone, a NAS accumulates one of these next to every
/// transfer that was ever interrupted.
///
/// Scoped hard: only this destination's own directory, only names that are
/// this file's name plus the exact suffix shape. Assumes no second process is
/// mid-write to the same destination path, which is already true — a link runs
/// one at a time.
fn sweep_orphans(destination: &dyn Backend, path: &Path) {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let directory = path.parent().unwrap_or(Path::new(""));
    let Ok(entries) = destination.read_dir(directory) else {
        return;
    };

    for entry in entries {
        if entry
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|candidate| is_orphan_temp(candidate, name))
        {
            tracing::debug!(path = %entry.path.display(), "removing an abandoned temporary file");
            let _ = destination.remove_file(&entry.path);
        }
    }
}

/// What discarding an interrupted run removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Discarded {
    /// Operations marked failed rather than left claiming to be in flight.
    pub operations: u64,
    /// Total size of the files those operations were carrying.
    ///
    /// What the transfer was *for*, not what was reclaimed: how much of each
    /// partial had landed is only knowable by asking the destination before
    /// deleting, which is a round trip per file.
    pub bytes: u64,
}

/// Files the engine set aside at a destination, waiting for a decision.
///
/// Paths are relative to the quarantine directory, because that is where the
/// user thinks of them as living: `holiday.mp4`, not
/// `.tungstate-quarantine/holiday.mp4`.
///
/// Lives here rather than in a caller because this crate is the one that
/// *writes* quarantine — [`QUARANTINE_DIR`] and the layout beneath it are its
/// convention, and a second reader spelling them out again is a second thing
/// to get wrong. It takes the destination backend for the same reason the
/// engine does: a quarantined file is written through the backend, so it is
/// only reachable through the backend. Reading it with `std::fs` finds nothing
/// whenever the destination is not this machine.
///
/// # Errors
/// [`TransferError::Backend`] if the destination cannot be listed. Note what is
/// *not* an error: a destination that has never had a conflict has no
/// quarantine directory, and that is the ordinary case of nothing set aside.
pub fn quarantined(destination: &dyn Backend) -> Result<Vec<PathBuf>> {
    let root = Path::new(QUARANTINE_DIR);

    // Asked by listing the destination rather than by stat'ing the directory,
    // so "nothing is set aside" and "the destination cannot be reached" stay
    // different answers. Reporting an unreachable NAS as an empty quarantine
    // is precisely the failure this function exists not to have.
    let present = destination
        .read_dir(Path::new(""))?
        .iter()
        .any(|entry| entry.meta.is_dir && entry.path == root);
    if !present {
        return Ok(Vec::new());
    }

    let mut found: Vec<PathBuf> = walk::files_under(destination, root)?
        .into_iter()
        // The walk started at `root`, so every path is under it and the
        // fallback is unreachable; it is there so a future change to the walk
        // cannot turn this into a panic.
        .map(|file| {
            file.path
                .strip_prefix(root)
                .unwrap_or(&file.path)
                .to_path_buf()
        })
        .collect();
    found.sort();
    Ok(found)
}

/// Abandon an interrupted run: clear what it left behind, copy nothing.
///
/// The counterpart to resuming. Same cleanup the next run would have done,
/// without the transfer — for when the answer to "shall I finish this?" is no
/// and the part-copied file should stop occupying the destination.
///
/// # Errors
/// [`TransferError::Journal`] if the operations cannot be read or updated.
/// Failing to remove a partial is not an error: already gone is the outcome
/// the caller wanted.
pub fn discard(link: &Link, destination: &dyn Backend, journal: &Journal) -> Result<Discarded> {
    let interrupted = journal.incomplete_for_link(link.id)?;
    let mut discarded = Discarded::default();

    for op in &interrupted {
        clear_partial(destination, op);
        journal.finish(
            op.id,
            &Outcome::Failed {
                error: "interrupted; discarded at the user's request".to_string(),
            },
        )?;
        discarded.operations += 1;
        discarded.bytes += op.size.unwrap_or(0);
    }

    Ok(discarded)
}

/// How many random characters a backend's temporary suffix has.
///
/// `OpenDAL`'s `build_tmp_path_of` uses eight. Matching it exactly is what keeps
/// the sweep from touching a real file: `holiday.mp4.backup` has six
/// characters after the dot and survives, `holiday.mp4.k3xq9wpz` has eight and
/// does not.
const TEMP_SUFFIX_LENGTH: usize = 8;

/// Whether `candidate` is a temporary file a backend left beside `name`.
fn is_orphan_temp(candidate: &str, name: &str) -> bool {
    let Some(suffix) = candidate
        .strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('.'))
    else {
        return false;
    };
    suffix.len() == TEMP_SUFFIX_LENGTH && suffix.chars().all(|c| c.is_ascii_alphanumeric())
}

/// BLAKE3 of everything `backend` holds at `path`.
/// Hash a file, telling `watching` how far in it is after every chunk.
///
/// The reporter is a closure rather than a `&dyn Progress` because the two
/// callers count differently: a content check spans two files and wants one
/// bar across both, while a readback verification is its own thing. Returning
/// `false` from it abandons the read.
fn digest_watching(
    backend: &dyn Backend,
    path: &Path,
    mut watching: impl FnMut(u64) -> bool,
) -> Result<String> {
    let mut reader = backend.open_read(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; CHUNK];
    loop {
        let read = reader.read(&mut buffer).map_err(|e| TransferError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        if !watching(read as u64) {
            return Err(TransferError::Stopped {
                path: path.to_path_buf(),
            });
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn digest(backend: &dyn Backend, path: &Path) -> Result<String> {
    digest_watching(backend, path, |_| true)
}

impl Transfer<'_> {
    /// Deal with the original. Only ever called after a verified, journaled commit.
    fn apply_source_policy(&self, path: &Path) -> Result<()> {
        match self.link.source_policy {
            SourcePolicy::Keep => Ok(()),
            SourcePolicy::Delete => {
                self.source.remove_file(path)?;
                Ok(())
            }
            SourcePolicy::Trash => {
                // `trash::delete` drives the local desktop's trash and knows
                // nothing about a remote. Refusing here rather than trusting
                // the CLI's check means a link built any other way — the GUI, a
                // future API — cannot delete the wrong local path by accident.
                // Remote trash (`.tungstate-trash/`, DESIGN.md §4) is a later
                // slice.
                if self.link.source.is_remote() {
                    return Err(TransferError::TrashUnsupported {
                        path: path.to_path_buf(),
                    });
                }
                let full = self.link.source.path.join(path);
                trash::delete(&full).map_err(|source| TransferError::Trash { path: full, source })
            }
        }
    }

    fn record_already_present(&self, file: &walk::File, destination: &Path) -> Result<Original> {
        let op = self.journal.begin(&NewOp {
            kind: OpKind::Move,
            source: Some(Location::within(&self.link.source, file.path.clone())),
            destination: Some(Location::within(
                &self.link.destination,
                destination.to_path_buf(),
            )),
            size: Some(file.size),
            link: Some(self.link.name.clone()),
            link_id: Some(self.link.id),
        })?;
        self.journal.finish(
            op,
            &Outcome::Skipped {
                reason: "identical copy already at destination".to_string(),
            },
        )?;

        // A copy has nothing to decide: the original was always staying.
        if self.link.source_policy == SourcePolicy::Keep {
            return Ok(Original::Kept);
        }

        // The content is verifiably there, so the source policy *can* apply —
        // but applying it means deleting the user's other copy on the strength
        // of a hash this program computed, and that is worth asking about.
        let kept = lock(&self.resolver).identical(&Identical {
            path: file.path.clone(),
            size: file.size,
        });
        match kept {
            IdenticalAction::KeepOriginal => Ok(Original::Kept),
            IdenticalAction::DeleteOriginal => {
                self.apply_source_policy(&file.path)?;
                Ok(Original::Removed)
            }
        }
    }
}

enum Placement {
    /// Nothing at the destination name.
    Fresh,
    /// Same content already there.
    Identical,
    /// Something else already there.
    Conflicting,
}

/// Find an unused name beside `destination`, as `holiday-2.mp4`.
///
/// A method rather than a function because the claim has to be atomic. Asking
/// the backend whether a name is free is not enough when several files are in
/// flight: two workers can both find `holiday-2.mp4` unused and both take it,
/// and the loser's bytes are overwritten by the winner. The lock is held
/// across probe-and-claim so only one of them can ever leave with the name.
impl Transfer<'_> {
    fn disambiguate(&self, destination: &Path) -> PathBuf {
        let stem = destination
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let extension = destination
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();

        let mut claimed = lock(&self.claimed);
        for suffix in 2..10_000 {
            let candidate = destination.with_file_name(format!("{stem}-{suffix}{extension}"));
            if !claimed.contains(&candidate) && self.destination.stat(&candidate).is_err() {
                claimed.insert(candidate.clone());
                return candidate;
            }
        }
        // Vanishingly unlikely; fall back to something unique rather than looping.
        let fallback = destination.with_file_name(format!("{stem}-{}{extension}", now_suffix()));
        claimed.insert(fallback.clone());
        fallback
    }
}

fn now_suffix() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

/// The default cooldown: long enough that a file a browser is still writing is
/// left alone, short enough not to delay a drain noticeably.
#[must_use]
pub fn default_cooldown() -> Duration {
    Duration::from_secs(30)
}

#[cfg(test)]
mod tests;
