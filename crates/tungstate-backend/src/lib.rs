//! Storage abstraction: one interface describing "a place files live".
//!
//! Everything above this crate talks to storage only through [`Backend`], so the
//! transfer engine works over a mounted volume, FTP, or SFTP without knowing
//! which it has. [`local::LocalBackend`] is the implementation for local disks.
//!
//! Paths handed to a [`Backend`] are always relative to its root, and an
//! implementation must reject any path that tries to escape. That rule is what
//! makes "tungstate cannot touch anything outside the folder it governs" a
//! property of the code rather than a promise in a comment.

pub mod local;
pub mod walk;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Anything that can go wrong reading or writing through a [`Backend`].
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// The path tried to leave the backend root, typically via `..`.
    #[error("path `{0}` escapes the backend root")]
    PathEscapesRoot(PathBuf),

    /// The path was absolute; backends only accept paths relative to their root.
    #[error("path `{0}` must be relative to the backend root")]
    PathNotRelative(PathBuf),

    /// The backend's root is gone, or is no longer the storage it was.
    ///
    /// The dangerous case this exists for: a network volume unmounts, its mount
    /// point becomes an ordinary empty directory on the boot disk, and a drain
    /// cheerfully fills the disk it was emptying while deleting the originals.
    #[error("`{0}` is no longer reachable, or is not the storage it was")]
    RootUnreachable(PathBuf),

    /// A symlink lay on the path, and following it could leave the root.
    #[error("refusing to follow symlink `{0}`")]
    SymlinkNotFollowed(PathBuf),

    /// The remote refused the credentials it was given, or was given none.
    ///
    /// Separate from [`BackendError::Remote`] because it is the one remote
    /// failure a user can fix without reading a protocol error: the answer is
    /// always "re-enter the password for this connection".
    #[error("`{endpoint}` refused the credentials it was given")]
    Auth {
        /// The connection that refused, named as the user named it.
        endpoint: String,
    },

    /// A remote operation failed for a protocol-specific reason.
    // Carries the endpoint as a string rather than a path: "io error at
    // `inbox/a.mp4`" is unactionable when three connections have an `inbox`.
    #[error("`{operation}` failed on `{endpoint}`")]
    Remote {
        /// The connection it happened on, named as the user named it.
        endpoint: String,
        /// Which operation was being attempted.
        operation: &'static str,
        /// The underlying protocol failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// An underlying I/O failure, tagged with the path that caused it.
    // The path matters: "permission denied" partway through a 4000-file drain is
    // unactionable without knowing which file refused.
    #[error("io error at `{path}`")]
    Io {
        /// Full path being operated on when the failure occurred.
        path: PathBuf,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
}

/// Result alias so signatures read `Result<Meta>` rather than spelling out the error.
pub type Result<T> = std::result::Result<T, BackendError>;

/// What a backend knows about one file or directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    /// Size in bytes. Meaningless for directories.
    pub len: u64,
    /// True for a directory.
    pub is_dir: bool,
    /// True for a symbolic link. Links are described, never followed.
    pub is_symlink: bool,
    // Not every filesystem or protocol reports this; FTP is especially vague. An
    // Option forces callers to handle its absence at compile time.
    /// Last modification time, where the backend can report one.
    pub modified: Option<SystemTime>,
    /// Which file on the storage this is, where the backend can say.
    ///
    /// Two paths with one identity are two *names* for one file, not two
    /// files: a hard link. The duplicate pass has to know, or it reports a
    /// duplicate that would reclaim nothing when it is dealt with
    /// (DESIGN §9). `None` wherever the question has no answer, which is
    /// every remote backend.
    pub identity: Option<String>,
}

/// One entry from [`Backend::read_dir`], carrying its path and metadata together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Path relative to the backend root, so it can be fed straight back in.
    pub path: PathBuf,
    /// Metadata for this entry.
    pub meta: Meta,
}

/// What a particular storage location can actually do.
///
/// These describe the filesystem or protocol, not the operating system. A
/// case-sensitive volume on macOS and a FAT stick on Linux both defy the
/// obvious guess, so implementations should discover these rather than assume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Four answers about one storage location, each a yes or a no. They are
/// independent facts rather than a state, so a struct of flags is what they
/// are; grouping them into enums would only hide that.
#[allow(clippy::struct_excessive_bools)]
pub struct Capabilities {
    /// Rename is atomic, so a commit cannot be observed half-done.
    pub atomic_rename: bool,
    /// Hard links are supported, which dedup can use to reclaim space.
    pub hard_links: bool,
    /// Paths differing only in case refer to different files.
    pub case_sensitive: bool,
    /// The bytes live on another machine, reached over a network.
    ///
    /// True for a mounted NAS as well as for a connection: a share mounted in
    /// Finder looks like an ordinary folder path, and reading a file on it
    /// still pulls every byte across the network. The duplicate pass samples
    /// rather than reads in full when this is set.
    pub networked: bool,
}

/// A cheap identifier for a backend's root.
///
/// Compared before each file so a volume swapped underneath a running transfer
/// is noticed before anything is written to the wrong disk.
///
/// On Unix this carries the filesystem's device id, which distinguishes a
/// mounted NAS from the boot disk even when the mount point still exists.
/// Windows exposes no equivalent on stable Rust, so there it degrades to a
/// reachability check, which still catches the mount point disappearing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootToken {
    /// Filesystem identity, where the platform exposes one.
    pub device: Option<u64>,
}

/// A sink for a file being written, which must be finished explicitly.
///
/// `flush` is not enough. On a local file it does nothing at all, because
/// `write_all` has already reached the kernel and the bytes are sitting in the
/// page cache; a power cut then loses them. Remote backends have the mirror
/// problem: a protocol write is not committed until the stream is closed.
///
/// [`WriteFinish::finish`] consumes the writer so it cannot be used afterwards,
/// which makes "did you commit this?" a compile-time question.
pub trait WriteFinish: Write + Send {
    /// Commit everything written durably, then close.
    ///
    /// # Errors
    /// [`BackendError::Io`] if the data cannot be committed. The destination
    /// must be treated as incomplete if this fails.
    fn finish(self: Box<Self>) -> Result<()>;
}

/// A place files live.
///
/// All paths are relative to the backend's root. Implementations must reject
/// absolute paths and any path containing `..`.
pub trait Backend: Send + Sync {
    /// What this storage location supports.
    fn capabilities(&self) -> Capabilities;

    /// Confirm the root is reachable, and identify which storage it is.
    ///
    /// # Errors
    /// [`BackendError::RootUnreachable`] if the root is missing or is not a
    /// directory.
    fn root_token(&self) -> Result<RootToken>;

    /// Read metadata for `path` without following symlinks.
    ///
    /// # Errors
    /// [`BackendError::PathEscapesRoot`] or [`BackendError::PathNotRelative`] if
    /// `path` is not a safe relative path, [`BackendError::Io`] if it cannot be
    /// read, including when it does not exist.
    fn stat(&self, path: &Path) -> Result<Meta>;

    /// List the immediate children of the directory at `path`.
    ///
    /// Deliberately one level deep and returning a `Vec`: a single directory is
    /// bounded, so collecting it is honest. Recursive walks need streaming.
    ///
    /// # Errors
    /// As [`Backend::stat`], plus [`BackendError::Io`] if `path` is not a
    /// readable directory.
    fn read_dir(&self, path: &Path) -> Result<Vec<Entry>>;

    /// Open `path` for streaming reads.
    ///
    /// Returns a boxed reader rather than bytes so a 40 GB video is never held
    /// in memory.
    ///
    /// # Errors
    /// As [`Backend::stat`], plus [`BackendError::Io`] if the file cannot be opened.
    fn open_read(&self, path: &Path) -> Result<Box<dyn Read + Send>>;

    /// Read at most `len` bytes from the start of `path`.
    ///
    /// What the classifier's cost tiers rest on: "sniff the media type" is
    /// 8 KiB, not the file. Defaulted rather than required, because a backend
    /// that cannot ask the far side for a range still answers correctly by
    /// truncating, and every test double keeps compiling. `LocalBackend` and
    /// the `OpenDAL` adapter override it with a real ranged read.
    ///
    /// # Errors
    /// As [`Backend::open_read`].
    fn read_prefix(&self, path: &Path, len: u64) -> Result<Vec<u8>> {
        let mut reader = self.open_read(path)?.take(len);
        let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0).min(1 << 20));
        reader
            .read_to_end(&mut bytes)
            .map_err(|source| BackendError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(bytes)
    }

    /// Set a file's modification time, where the storage allows it.
    ///
    /// `Ok(false)` means "cannot here", which is not a failure: a sync carries
    /// a member's time across where it can and says so where it cannot, and
    /// never depends on it, since each member is only compared with itself.
    /// Defaulted, as `read_prefix` is, so every backend keeps compiling.
    ///
    /// # Errors
    /// [`BackendError::Io`] if the storage allows it and it still failed.
    fn set_modified(&self, path: &Path, at: std::time::SystemTime) -> Result<bool> {
        let _ = (path, at);
        Ok(false)
    }

    /// Create or truncate `path` and open it for streaming writes.
    ///
    /// The caller must call [`WriteFinish::finish`]; dropping the writer without
    /// it leaves the destination unreliable.
    ///
    /// # Errors
    /// As [`Backend::stat`], plus [`BackendError::Io`] if the file cannot be created.
    fn create_write(&self, path: &Path) -> Result<Box<dyn WriteFinish>>;

    /// Move `from` to `to`, replacing `to` if it exists.
    ///
    /// # Errors
    /// As [`Backend::stat`] for either path, plus [`BackendError::Io`] if the
    /// rename fails.
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;

    /// Delete the file at `path`.
    ///
    /// # Errors
    /// As [`Backend::stat`], plus [`BackendError::Io`] if the file cannot be removed.
    fn remove_file(&self, path: &Path) -> Result<()>;

    /// Delete the directory at `path`, which must be empty.
    ///
    /// # Errors
    /// As [`Backend::stat`], plus [`BackendError::Io`] if the directory is
    /// missing or not empty.
    fn remove_dir(&self, path: &Path) -> Result<()>;

    /// Create the directory at `path` and any missing parents.
    ///
    /// # Errors
    /// As [`Backend::stat`], plus [`BackendError::Io`] if creation fails.
    fn create_dir_all(&self, path: &Path) -> Result<()>;

    /// Where `path` really is on this machine, if it is on this machine.
    ///
    /// `None` for everything remote, which is the default, so a backend only
    /// answers this if it can. The desktop trash is the one caller: it drives
    /// the local Finder or file manager and knows nothing about an FTP
    /// server, so an op that needs it has to be refused rather than guessed
    /// at.
    ///
    /// # Errors
    /// As [`Backend::stat`] when `path` is not a safe relative path.
    fn on_this_machine(&self, path: &Path) -> Result<Option<PathBuf>> {
        let _ = path;
        Ok(None)
    }
}
