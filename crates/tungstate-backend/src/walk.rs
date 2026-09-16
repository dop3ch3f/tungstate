//! Depth-first walking of a backend, one directory's listing at a time.
//!
//! [`Backend::read_dir`] is deliberately one level deep, and says so: a single
//! directory is bounded, so collecting it is honest, and recursive walks need
//! streaming. This is that streaming counterpart, so a million-file tree costs
//! one directory's listing plus the pending stack rather than the whole tree.
//!
//! It lives beside the trait because it needs nothing else. What to skip
//! arrives as a closure, never as a pattern type: this crate describes "a place
//! files live" and must not learn the policy language.

use std::path::{Path, PathBuf};

use crate::{Backend, Entry, Result};

/// Every entry beneath a root, depth first.
///
/// Symlinks are yielded and never descended. [`Backend`] already refuses to
/// read through one, and a walk that followed them would leave the tree it was
/// pointed at.
pub struct Walk<'a> {
    backend: &'a dyn Backend,
    /// Directories found and not yet listed. The explicit stack is what keeps
    /// this iterative: a recursive walk would hold one stack frame per level
    /// and a deep tree would end the process rather than return an error.
    pending: Vec<PathBuf>,
    /// The directory currently being handed out, one entry at a time.
    buffer: std::vec::IntoIter<Entry>,
    #[allow(clippy::type_complexity)]
    prune: Option<Box<dyn Fn(&Entry) -> bool + 'a>>,
}

impl<'a> Walk<'a> {
    /// Walk everything beneath `root`, which is itself relative to the backend
    /// root. An empty path means the whole backend.
    #[must_use]
    pub fn new(backend: &'a dyn Backend, root: &Path) -> Self {
        Self {
            backend,
            pending: vec![root.to_path_buf()],
            buffer: Vec::new().into_iter(),
            prune: None,
        }
    }

    /// Do not descend into a directory this returns `false` for.
    ///
    /// The directory itself is still yielded; only its contents are skipped.
    /// That distinction matters to a caller deciding whether a directory is
    /// empty: a pruned directory that vanished from the walk would look empty
    /// to everyone downstream.
    #[must_use]
    pub fn prune(mut self, keep_going: impl Fn(&Entry) -> bool + 'a) -> Self {
        self.prune = Some(Box::new(keep_going));
        self
    }

    fn descend(&self, entry: &Entry) -> bool {
        entry.meta.is_dir
            && !entry.meta.is_symlink
            && self.prune.as_ref().is_none_or(|keep| keep(entry))
    }
}

impl Iterator for Walk<'_> {
    type Item = Result<Entry>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(entry) = self.buffer.next() {
                if self.descend(&entry) {
                    self.pending.push(entry.path.clone());
                }
                return Some(Ok(entry));
            }
            let directory = self.pending.pop()?;
            match self.backend.read_dir(&directory) {
                Ok(entries) => self.buffer = entries.into_iter(),
                // Reported rather than swallowed, and the walk stops. A caller
                // that wants to carry on past an unreadable subtree can, since
                // this is one item in a stream rather than a failed `Vec`.
                Err(error) => return Some(Err(error)),
            }
        }
    }
}

/// Every regular file beneath `root`, collected.
///
/// The convenience form for callers that want the whole list anyway — sorting
/// by size needs it, and a plan needs it. Bounded by file count, not by
/// content, so even a tree of very large files is cheap to list.
///
/// # Errors
/// Whatever [`Backend::read_dir`] reports for any directory in the tree.
pub fn files_under(backend: &dyn Backend, root: &Path) -> Result<Vec<Entry>> {
    // An `Err` has to survive the filter or `collect` never sees it, and an
    // unreadable subtree would read as an empty one.
    Walk::new(backend, root)
        .filter(|entry| match entry {
            Ok(entry) => !entry.meta.is_dir && !entry.meta.is_symlink,
            Err(_) => true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::LocalBackend;

    /// A tree with a directory worth refusing to enter.
    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("keep/deeper")).unwrap();
        std::fs::create_dir_all(root.join("skip/inside")).unwrap();
        std::fs::write(root.join("top.txt"), b"1").unwrap();
        std::fs::write(root.join("keep/a.txt"), b"2").unwrap();
        std::fs::write(root.join("keep/deeper/b.txt"), b"3").unwrap();
        std::fs::write(root.join("skip/c.txt"), b"4").unwrap();
        std::fs::write(root.join("skip/inside/d.txt"), b"5").unwrap();
        dir
    }

    fn names(entries: &[Entry]) -> Vec<String> {
        let mut names: Vec<String> = entries
            .iter()
            .map(|e| e.path.to_string_lossy().replace('\\', "/"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_walk_finds_everything_at_every_depth() {
        let dir = tree();
        let backend = LocalBackend::new(dir.path().to_path_buf());
        let found: Vec<Entry> = Walk::new(&backend, Path::new(""))
            .collect::<Result<_>>()
            .expect("the walk succeeds");
        assert_eq!(
            names(&found),
            [
                "keep",
                "keep/a.txt",
                "keep/deeper",
                "keep/deeper/b.txt",
                "skip",
                "skip/c.txt",
                "skip/inside",
                "skip/inside/d.txt",
                "top.txt",
            ]
        );
    }

    #[test]
    fn files_under_leaves_out_the_directories() {
        let dir = tree();
        let backend = LocalBackend::new(dir.path().to_path_buf());
        let found = files_under(&backend, Path::new("")).expect("the walk succeeds");
        assert_eq!(
            names(&found),
            [
                "keep/a.txt",
                "keep/deeper/b.txt",
                "skip/c.txt",
                "skip/inside/d.txt",
                "top.txt",
            ]
        );
    }

    #[test]
    fn a_pruned_directory_is_still_reported_but_never_entered() {
        // The distinction the `prune` docstring turns on. A caller deciding
        // whether a directory is empty has to see it; what it must not see is
        // what is inside.
        let dir = tree();
        let backend = LocalBackend::new(dir.path().to_path_buf());
        let found: Vec<Entry> = Walk::new(&backend, Path::new(""))
            .prune(|entry| entry.path != Path::new("skip"))
            .collect::<Result<_>>()
            .expect("the walk succeeds");
        assert_eq!(
            names(&found),
            [
                "keep",
                "keep/a.txt",
                "keep/deeper",
                "keep/deeper/b.txt",
                "skip",
                "top.txt",
            ]
        );
    }

    #[test]
    fn a_walk_can_start_below_the_root() {
        let dir = tree();
        let backend = LocalBackend::new(dir.path().to_path_buf());
        let found = files_under(&backend, Path::new("keep")).expect("the walk succeeds");
        assert_eq!(names(&found), ["keep/a.txt", "keep/deeper/b.txt"]);
    }

    #[test]
    fn an_unreadable_root_is_an_error_rather_than_an_empty_walk() {
        // "Nothing is there" and "I could not look" are different answers, and
        // a walk that collapsed them would let a vanished mount read as a tidy
        // folder.
        let dir = tempfile::tempdir().expect("a temp dir");
        let backend = LocalBackend::new(dir.path().join("not-here"));
        assert!(files_under(&backend, Path::new("")).is_err());
    }
}
