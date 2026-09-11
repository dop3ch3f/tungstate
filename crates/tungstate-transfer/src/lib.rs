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
mod walk;

pub use conflict::{Conflict, ConflictResolver, Decision, FixedResolver, InteractiveResolver};

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

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
    /// Already present at the destination with identical content.
    AlreadyPresent,
    /// Left alone, with the reason.
    Skipped(SkipReason),
    /// Parked under the quarantine directory.
    Quarantined,
    /// Could not be transferred. The original was left untouched.
    Failed,
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

/// Told about each file as it is dealt with, so a caller can show progress.
pub trait Progress {
    /// A file is about to be transferred.
    fn starting(&mut self, path: &Path, size: u64);
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
fn gather(source: &dyn Backend, chosen: &[PathBuf]) -> Result<Vec<walk::File>> {
    let mut files = Vec::new();
    for path in chosen {
        let meta = source.stat(path)?;
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
    resolver: &'a mut dyn ConflictResolver,
    progress: &'a mut dyn Progress,
    // Checked between files, never mid-file: stopping partway through a copy
    // would leave a partial, and the next run would redo it anyway.
    cancel: Option<Arc<AtomicBool>>,
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
            resolver,
            progress,
            cancel: None,
        }
    }

    /// Stop cleanly when `flag` is set.
    ///
    /// Honoured between files rather than mid-copy, so a cancelled run leaves
    /// completed transfers committed and nothing half-written.
    #[must_use]
    pub fn cancellable(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel = Some(flag);
        self
    }

    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
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
        let files = walk::files(self.source)?;
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
        let mut files = Vec::new();
        for path in chosen {
            let meta = self.source.stat(path)?;
            if meta.is_dir {
                files.extend(walk::files_under(self.source, path)?);
            } else if !meta.is_symlink {
                files.push(walk::File {
                    path: path.clone(),
                    size: meta.len,
                    modified: meta.modified,
                });
            }
        }
        self.carry(files)
    }

    fn carry(&mut self, mut files: Vec<walk::File>) -> Result<Summary> {
        // Pin what the destination is before anything moves, so a volume
        // swapped underneath us is detectable rather than silently written to.
        let anchor = self.destination.root_token()?;

        let mut summary = Summary {
            recovered: self.recover()?,
            ..Summary::default()
        };

        walk::sort(&mut files, self.link.order);

        for file in files {
            if self.cancelled() {
                tracing::info!("stopping at the user's request");
                summary.cancelled = true;
                break;
            }

            // Checked per file rather than once: a NAS can drop out at any
            // point, and every file after that would otherwise be written to
            // whatever now sits at that path, then have its original deleted.
            match self.destination.root_token() {
                Ok(token) if token == anchor => {}
                _ => {
                    tracing::error!("destination is no longer the storage we started with");
                    summary.destination_lost = true;
                    break;
                }
            }
            // One unreadable file must not abandon the other three thousand.
            // The failure is journaled, reported, and the drain carries on;
            // `failures` is what the caller shows at the end.
            let outcome = match self.transfer_one(&file) {
                Ok(outcome) => outcome,
                Err(error) => {
                    tracing::warn!(path = %file.path.display(), %error, "transfer failed");
                    summary.failed += 1;
                    summary.failures.push(Failure {
                        path: file.path.clone(),
                        reason: explain(&error),
                    });
                    FileOutcome::Failed
                }
            };
            self.progress.finished(&file.path, outcome);

            match outcome {
                FileOutcome::Transferred => {
                    summary.transferred += 1;
                    summary.bytes += file.size;
                }
                FileOutcome::AlreadyPresent => summary.already_present += 1,
                FileOutcome::Skipped(_) => summary.skipped += 1,
                FileOutcome::Quarantined => summary.quarantined += 1,
                // Already counted above; the source was left untouched.
                FileOutcome::Failed => {}
            }
        }

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
    fn recover(&mut self) -> Result<u64> {
        let interrupted = self.journal.incomplete_for_link(self.link.id)?;

        let atomic = self.destination.capabilities().atomic_rename;

        for op in &interrupted {
            if let Some(destination) = &op.destination {
                if atomic {
                    let partial = temp_name(&destination.path, op.id);
                    // Already gone is the common case and not an error.
                    let _ = self.destination.remove_file(&partial);
                } else {
                    // Nothing was written to a temp name, so the only partial
                    // there can be is under the real name. Removing it is safe
                    // because the source has not been touched: `copy_to`
                    // applies the source policy only after the journal commit
                    // this operation never reached.
                    let _ = self.destination.remove_file(&destination.path);
                    self.sweep_orphans(&destination.path);
                }
            }
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
    fn sweep_orphans(&self, destination: &Path) {
        let Some(name) = destination.file_name().and_then(|n| n.to_str()) else {
            return;
        };
        let directory = destination.parent().unwrap_or(Path::new(""));
        let Ok(entries) = self.destination.read_dir(directory) else {
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
                let _ = self.destination.remove_file(&entry.path);
            }
        }
    }

    fn transfer_one(&mut self, file: &walk::File) -> Result<FileOutcome> {
        self.progress.starting(&file.path, file.size);

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
            Placement::Identical => {
                self.record_already_present(file, &destination)?;
                Ok(FileOutcome::AlreadyPresent)
            }
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

        let here = digest(self.source, &file.path)?;
        let there = digest(self.destination, destination)?;
        if here == there {
            Ok(Placement::Identical)
        } else {
            Ok(Placement::Conflicting)
        }
    }

    fn resolve_conflict(&mut self, file: &walk::File, destination: &Path) -> Result<FileOutcome> {
        let existing = self.destination.stat(destination)?;
        let action = self.resolver.resolve(&Conflict {
            path: file.path.clone(),
            incoming_size: file.size,
            existing_size: existing.len,
        });

        match action {
            ConflictAction::Skip => Ok(FileOutcome::Skipped(SkipReason::Conflict)),

            ConflictAction::Rename => {
                let renamed = disambiguate(destination, self.destination);
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
    fn copy_to(&mut self, file: &walk::File, destination: &Path) -> Result<()> {
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
        let mut reader = self.source.open_read(source)?;
        let mut writer = self.destination.create_write(destination)?;
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
        }

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
fn digest(backend: &dyn Backend, path: &Path) -> Result<String> {
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
    }
    Ok(hasher.finalize().to_hex().to_string())
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

    fn record_already_present(&self, file: &walk::File, destination: &Path) -> Result<()> {
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
        // The content is safely there, so the source policy still applies.
        self.apply_source_policy(&file.path)
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
fn disambiguate(destination: &Path, backend: &dyn Backend) -> PathBuf {
    let stem = destination
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let extension = destination
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    for suffix in 2..10_000 {
        let candidate = destination.with_file_name(format!("{stem}-{suffix}{extension}"));
        if backend.stat(&candidate).is_err() {
            return candidate;
        }
    }
    // Vanishingly unlikely; fall back to something unique rather than looping.
    destination.with_file_name(format!("{stem}-{}{extension}", now_suffix()))
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
