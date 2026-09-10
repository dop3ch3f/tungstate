//! Depth-first listing of a backend, and the orderings a link can ask for.
//!
//! Lives here rather than on `Backend` because ordering by size needs the whole
//! list anyway. Bounded by file count, not by content, so a drain of very large
//! files is cheap to plan. Slice 6 adds a streaming walk for million-file trees.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tungstate_backend::Backend;
use tungstate_journal::Order;

use crate::Result;

/// One file found in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct File {
    /// Path relative to the backend root.
    pub(crate) path: PathBuf,
    /// Size in bytes.
    pub(crate) size: u64,
    /// Last modification time, where the backend reports one.
    pub(crate) modified: Option<SystemTime>,
}

/// Every regular file under the backend root, depth first.
///
/// Symlinks are not followed. `Backend` already refuses to read through one, and
/// a drain that followed links would copy content from outside the folder it was
/// pointed at.
pub(crate) fn files(backend: &dyn Backend) -> Result<Vec<File>> {
    files_under(backend, Path::new(""))
}

/// Every regular file beneath `root`, which is itself relative to the backend root.
pub(crate) fn files_under(backend: &dyn Backend, root: &Path) -> Result<Vec<File>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        for entry in backend.read_dir(&directory)? {
            if entry.meta.is_symlink {
                tracing::debug!(path = %entry.path.display(), "skipping symlink");
                continue;
            }
            if entry.meta.is_dir {
                pending.push(entry.path);
            } else {
                found.push(File {
                    path: entry.path,
                    size: entry.meta.len,
                    modified: entry.meta.modified,
                });
            }
        }
    }

    Ok(found)
}

/// Put files in the order the link asked for.
pub(crate) fn sort(files: &mut [File], order: Order) {
    match order {
        // Biggest first, so space comes back soonest. The point of a drain.
        Order::LargestFirst => {
            files.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
        }
        Order::SmallestFirst => {
            files.sort_by(|a, b| a.size.cmp(&b.size).then_with(|| a.path.cmp(&b.path)));
        }
        // Files with no reported time sort last rather than being dropped.
        Order::OldestFirst => files.sort_by(|a, b| match (a.modified, b.modified) {
            (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.path.cmp(&b.path)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.path.cmp(&b.path),
        }),
        // Path order, because the walk itself has no meaningful order and a
        // reproducible run is worth more than an arbitrary one.
        Order::Discovered => files.sort_by(|a, b| a.path.cmp(&b.path)),
    }
}

/// Remove directories under `root` that the walk left empty.
///
/// Only removes what is genuinely empty, so a directory the user still has
/// something in is never touched.
pub(crate) fn prune_empty(backend: &dyn Backend, root: &Path) -> Result<u64> {
    let mut removed = 0;
    let mut directories = Vec::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        for entry in backend.read_dir(&directory)? {
            if entry.meta.is_dir && !entry.meta.is_symlink {
                pending.push(entry.path.clone());
                directories.push(entry.path);
            }
        }
    }

    // Deepest first, so emptying a child lets its parent go too.
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        if backend.read_dir(&directory).is_ok_and(|e| e.is_empty())
            && backend.remove_dir(&directory).is_ok()
        {
            removed += 1;
        }
    }

    Ok(removed)
}
