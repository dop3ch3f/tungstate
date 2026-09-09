//! [`Backend`] over the local filesystem.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Backend, BackendError, Capabilities, Entry, Meta, Result};

/// Prefix for probe files, so a leftover is obviously ours and obviously junk.
const PROBE_PREFIX: &str = ".tungstate-probe";

/// Distinguishes concurrent probes within one process.
static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);

/// A probe filename no other probe will pick.
///
/// Two backends rooted at the same directory would otherwise race on a fixed
/// name, and one deleting the other's file mid-probe reports a false negative.
fn probe_name(kind: &str) -> String {
    let seq = PROBE_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{PROBE_PREFIX}-{kind}-{}-{seq}", std::process::id())
}

/// Turn a caller's relative path into a real filesystem path, or refuse.
///
/// This is the security boundary of the whole product: every [`Backend`] method
/// funnels through it, so a path that escapes the root cannot reach `std::fs`.
///
/// Uses component inspection rather than `canonicalize`, which requires the file
/// to already exist and so cannot validate the path of a file about to be created.
fn resolve(root: &Path, path: &Path) -> Result<PathBuf> {
    // Deliberately no `is_absolute()` check: on Windows `/etc/passwd` reports
    // false, having no drive prefix, yet `join` still discards the root and
    // yields `C:\etc\passwd`. Inspecting components catches both platforms with
    // one rule. Matching exhaustively also means a future Component variant
    // becomes a compile error rather than a silent hole.
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => {
                return Err(BackendError::PathNotRelative(path.to_path_buf()));
            }
            Component::ParentDir => {
                return Err(BackendError::PathEscapesRoot(path.to_path_buf()));
            }
        }
    }
    Ok(root.join(path))
}

/// Build a [`BackendError::Io`] closure that tags the failure with `path`.
fn io_at(path: &Path) -> impl FnOnce(std::io::Error) -> BackendError {
    let path = path.to_path_buf();
    |source| BackendError::Io { path, source }
}

/// A [`Backend`] rooted at a directory on the local filesystem.
#[derive(Debug)]
pub struct LocalBackend {
    root: PathBuf,
    // Probing writes files, so it is deferred until something actually asks.
    // OnceLock gives us that from behind `&self` without a Mutex, and stays Sync.
    capabilities: OnceLock<Capabilities>,
}

impl LocalBackend {
    /// Open a backend rooted at `root`.
    ///
    /// Does no I/O and cannot fail; the directory is not required to exist yet,
    /// and capabilities are probed on first use.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            capabilities: OnceLock::new(),
        }
    }

    /// The directory this backend is rooted at.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Discover what the filesystem under `root` supports by trying it.
///
/// Asking the operating system would be wrong: a case-sensitive APFS volume and
/// a FAT stick on Linux both contradict the obvious guess. A probe that fails
/// yields `false`, because "cannot hard-link here" is an answer, not an error.
fn probe(root: &Path) -> Capabilities {
    Capabilities {
        atomic_rename: true,
        hard_links: probe_hard_links(root),
        case_sensitive: probe_case_sensitive(root),
    }
}

fn probe_hard_links(root: &Path) -> bool {
    let name = probe_name("link");
    let source = root.join(&name);
    let link = root.join(format!("{name}-dst"));

    if std::fs::write(&source, b"probe").is_err() {
        return false;
    }
    let supported = std::fs::hard_link(&source, &link).is_ok();

    let _ = std::fs::remove_file(&source);
    let _ = std::fs::remove_file(&link);
    supported
}

fn probe_case_sensitive(root: &Path) -> bool {
    let name = probe_name("case");
    let lower = root.join(&name);
    let upper = root.join(name.to_uppercase());

    if std::fs::write(&lower, b"probe").is_err() {
        return false;
    }
    // If the uppercase spelling finds the file we just wrote in lowercase, the
    // filesystem folded the case and is therefore insensitive.
    let sensitive = !upper.exists();

    let _ = std::fs::remove_file(&lower);
    sensitive
}

impl Backend for LocalBackend {
    fn capabilities(&self) -> Capabilities {
        *self.capabilities.get_or_init(|| probe(&self.root))
    }

    fn stat(&self, path: &Path) -> Result<Meta> {
        let full = resolve(&self.root, path)?;
        // symlink_metadata rather than metadata: a link is described, not followed,
        // so a link pointing outside the root cannot be used to read past it.
        let md = std::fs::symlink_metadata(&full).map_err(io_at(&full))?;
        Ok(Meta {
            len: md.len(),
            is_dir: md.is_dir(),
            is_symlink: md.file_type().is_symlink(),
            modified: md.modified().ok(),
        })
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<Entry>> {
        let full = resolve(&self.root, path)?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&full).map_err(io_at(&full))? {
            let entry = entry.map_err(io_at(&full))?;
            let relative = path.join(entry.file_name());
            entries.push(Entry {
                meta: self.stat(&relative)?,
                path: relative,
            });
        }
        Ok(entries)
    }

    fn open_read(&self, path: &Path) -> Result<Box<dyn std::io::Read + Send>> {
        let full = resolve(&self.root, path)?;
        let file = std::fs::File::open(&full).map_err(io_at(&full))?;
        Ok(Box::new(file))
    }

    fn create_write(&self, path: &Path) -> Result<Box<dyn std::io::Write + Send>> {
        let full = resolve(&self.root, path)?;
        let file = std::fs::File::create(&full).map_err(io_at(&full))?;
        Ok(Box::new(file))
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let from_full = resolve(&self.root, from)?;
        let to_full = resolve(&self.root, to)?;
        std::fs::rename(&from_full, &to_full).map_err(io_at(&from_full))
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        let full = resolve(&self.root, path)?;
        std::fs::remove_file(&full).map_err(io_at(&full))
    }

    fn remove_dir(&self, path: &Path) -> Result<()> {
        let full = resolve(&self.root, path)?;
        std::fs::remove_dir(&full).map_err(io_at(&full))
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        let full = resolve(&self.root, path)?;
        std::fs::create_dir_all(&full).map_err(io_at(&full))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Every test gets its own directory, so the suite is safe to run in parallel.
    fn backend() -> (tempfile::TempDir, Box<dyn Backend>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let backend = Box::new(LocalBackend::new(dir.path().to_path_buf()));
        (dir, backend)
    }

    #[test]
    fn rejects_paths_that_escape_the_root() {
        let root = Path::new("/tmp/example-root");

        for escape in ["../etc/passwd", "a/../../b", "a/b/../../../c"] {
            assert!(
                matches!(
                    resolve(root, Path::new(escape)),
                    Err(BackendError::PathEscapesRoot(_))
                ),
                "`{escape}` should have been rejected"
            );
        }

        for rooted in ["/etc/passwd", "/"] {
            assert!(
                matches!(
                    resolve(root, Path::new(rooted)),
                    Err(BackendError::PathNotRelative(_))
                ),
                "`{rooted}` should have been rejected"
            );
        }
    }

    #[test]
    fn no_accepted_path_can_land_outside_the_root() {
        // The property that actually matters, asserted directly rather than via
        // an error variant. Windows CI caught the earlier version of this: a
        // leading `/` is not `is_absolute()` there, but join still escaped.
        let root = Path::new("/tmp/example-root");

        for candidate in [
            "a/b.txt",
            "./a",
            "",
            "..",
            "/",
            "/etc/passwd",
            "a/../../b",
            "C:/windows",
        ] {
            if let Ok(resolved) = resolve(root, Path::new(candidate)) {
                assert!(
                    resolved.starts_with(root),
                    "`{candidate}` resolved to `{}`, outside the root",
                    resolved.display()
                );
            }
        }
    }

    #[test]
    fn accepts_ordinary_relative_paths() {
        let root = Path::new("/tmp/example-root");
        assert_eq!(
            resolve(root, Path::new("a/b.txt")).unwrap(),
            Path::new("/tmp/example-root/a/b.txt")
        );
        assert_eq!(
            resolve(root, Path::new("./a")).unwrap(),
            Path::new("/tmp/example-root/./a")
        );
    }

    #[test]
    fn lists_and_stats_the_root_itself() {
        // The empty path means "the root". Slice 3's drain starts here, so this
        // is the most-travelled call in the crate.
        let (_dir, backend) = backend();
        drop(backend.create_write(Path::new("a.mp4")).unwrap());

        let entries = backend.read_dir(Path::new("")).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, Path::new("a.mp4"));

        assert!(backend.stat(Path::new("")).unwrap().is_dir);
    }

    #[test]
    fn round_trips_a_file_through_the_trait_object() {
        let (_dir, backend) = backend();

        backend.create_dir_all(Path::new("sub")).unwrap();

        // Scoped so the writer is dropped, closing the handle. Windows refuses
        // other operations on a file that still has one open.
        {
            let mut writer = backend.create_write(Path::new("sub/video.mp4")).unwrap();
            writer.write_all(b"hello").unwrap();
        }

        let meta = backend.stat(Path::new("sub/video.mp4")).unwrap();
        assert_eq!(meta.len, 5);
        assert!(!meta.is_dir);
        assert!(!meta.is_symlink);

        let entries = backend.read_dir(Path::new("sub")).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, Path::new("sub/video.mp4"));

        let mut contents = String::new();
        backend
            .open_read(Path::new("sub/video.mp4"))
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert_eq!(contents, "hello");
    }

    #[test]
    fn renames_and_removes() {
        let (_dir, backend) = backend();
        backend.create_dir_all(Path::new("d")).unwrap();
        drop(backend.create_write(Path::new("d/a")).unwrap());

        backend.rename(Path::new("d/a"), Path::new("d/b")).unwrap();
        assert!(backend.stat(Path::new("d/a")).is_err());
        assert!(backend.stat(Path::new("d/b")).is_ok());

        backend.remove_file(Path::new("d/b")).unwrap();
        backend.remove_dir(Path::new("d")).unwrap();
        assert!(backend.stat(Path::new("d")).is_err());
    }

    #[test]
    fn io_errors_carry_the_path_that_failed() {
        let (dir, backend) = backend();

        let error = backend.stat(Path::new("missing.txt")).unwrap_err();
        match error {
            BackendError::Io { path, .. } => assert_eq!(path, dir.path().join("missing.txt")),
            other => panic!("expected an Io error, got {other:?}"),
        }
    }

    #[test]
    fn probes_the_filesystem_without_leaving_anything_behind() {
        let (dir, backend) = backend();

        // Asserting a specific answer would be wrong: this Mac reports
        // case-insensitive, Linux CI reports case-sensitive, and both are correct.
        let first = backend.capabilities();
        assert_eq!(first, backend.capabilities(), "probe result must be cached");

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(
            leftovers.is_empty(),
            "probe left files behind: {leftovers:?}"
        );
    }

    #[test]
    fn probing_an_unwritable_root_answers_false_rather_than_panicking() {
        // "Cannot hard-link here" is a true answer, so a probe that fails to
        // write must degrade rather than blow up a caller who only wanted to read.
        let backend = LocalBackend::new(PathBuf::from("/tungstate-does-not-exist"));

        let capabilities = backend.capabilities();
        assert!(!capabilities.hard_links);
        assert!(!capabilities.case_sensitive);
    }
}
