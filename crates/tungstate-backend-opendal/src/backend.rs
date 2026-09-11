//! [`Backend`] over an `OpenDAL` [`Operator`].

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use opendal::{Buffer, ErrorKind, Operator, Reader, Writer};
use tungstate_backend::{
    Backend, BackendError, Capabilities, Entry, Meta, Result, RootToken, WriteFinish,
};

use crate::keys::remote_key;
use crate::runtime::dispatch;

/// Join two already-validated key fragments, skipping empties.
fn join(prefix: &str, relative: &str) -> String {
    match (prefix.is_empty(), relative.is_empty()) {
        (true, _) => relative.to_string(),
        (false, true) => prefix.to_string(),
        (false, false) => format!("{prefix}/{relative}"),
    }
}

/// The inverse of [`join`], for turning a listing back into caller-relative
/// paths.
fn strip_prefix<'a>(prefix: &str, key: &'a str) -> &'a str {
    if prefix.is_empty() {
        return key;
    }
    key.strip_prefix(prefix)
        .map_or(key, |rest| rest.trim_start_matches('/'))
}

/// A [`Backend`] over one `OpenDAL` operator.
///
/// The operator is rooted at the *connection*, and `prefix` is the path inside
/// it that this link end names. Keeping those apart is what makes
/// [`Backend::root_token`] mean "the storage is still there" rather than "that
/// particular subfolder exists": a destination folder nobody has created yet
/// is an ordinary first write, while an unmounted NAS still stops the drain.
#[derive(Debug, Clone)]
pub struct OpendalBackend {
    operator: Operator,
    /// The link end's path inside the connection. Empty means its root.
    prefix: String,
    /// How "is the place still there?" is answered. See [`Anchor`].
    anchor: Anchor,
    /// The connection name as the user typed it, for error messages. Three
    /// connections can all have an `inbox`; the path alone does not identify.
    endpoint: String,
}

/// How a backend answers "is the connection still there?".
///
/// This exists because of a trap. [`Backend::root_token`] is called once per
/// file, and `OpenDAL` short-circuits `stat("/")` to a synthetic directory
/// without touching the store at all (`layers/simulate.rs`). Using it would
/// leave tungstate's most important safety rail — notice a NAS unmounting
/// before writing the next file to the boot disk — silently answering "yes,
/// fine" forever.
#[derive(Debug, Clone)]
pub enum Anchor {
    /// A directory on this machine. One `stat` syscall, exactly as
    /// `LocalBackend` does it, and exactly as cheap.
    LocalDir(PathBuf),
    /// Somewhere only the protocol can reach. Listing the root is the cheapest
    /// thing `OpenDAL` offers that genuinely goes to the store; `limit` is only
    /// a hint and the lister is collected regardless. Once-per-file is fine for
    /// a small root and quadratic for a large one, so slice 4c has to revisit
    /// this when FTP makes it a real cost.
    Store,
}

impl OpendalBackend {
    /// Wrap an operator rooted at the connection, for the link end at `prefix`.
    ///
    /// `endpoint` is the connection's name, used only in error messages.
    #[must_use]
    pub fn new(operator: Operator, prefix: String, anchor: Anchor, endpoint: String) -> Self {
        Self {
            operator,
            prefix,
            anchor,
            endpoint,
        }
    }

    /// A caller's relative path as a key inside this link end.
    fn key(&self, path: &Path) -> Result<String> {
        Ok(join(&self.prefix, &remote_key(path)?))
    }

    /// The same, spelled as a directory. `OpenDAL` decides file-or-directory
    /// from the trailing slash, so this is not cosmetic.
    fn directory(&self, path: &Path) -> Result<String> {
        let joined = self.key(path)?;
        if joined.is_empty() {
            return Ok("/".to_string());
        }
        Ok(format!("{joined}/"))
    }

    /// The connection name this backend speaks to.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Map an `OpenDAL` failure onto the shared error type.
    ///
    /// `NotFound` deliberately becomes an ordinary [`BackendError::Io`]: the
    /// transfer engine reads "stat failed" as "nothing is there" in three
    /// places, and a protocol-flavoured error would still work but would read
    /// like a fault in a log where it is the normal case.
    fn failure(&self, operation: &'static str, path: &Path, error: opendal::Error) -> BackendError {
        if error.kind() == ErrorKind::NotFound {
            return BackendError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, error.to_string()),
            };
        }
        BackendError::Remote {
            endpoint: self.endpoint.clone(),
            operation,
            source: Box::new(error),
        }
    }

    /// `stat`, with the retry that `OpenDAL`'s file-or-directory convention needs.
    ///
    /// `OpenDAL` decides which one a key means from its trailing slash, and a
    /// caller of [`Backend::stat`] has no reason to know that. So a miss on the
    /// file spelling is retried as the directory spelling before giving up.
    fn stat_either(&self, path: &Path) -> Result<opendal::Metadata> {
        let key = self.key(path)?;
        if key.is_empty() {
            let directory = self.directory(path)?;
            return self.stat_key(&directory, path);
        }
        match self.stat_key(&key, path) {
            Ok(meta) => Ok(meta),
            Err(BackendError::Io { .. }) => {
                let directory = self.directory(path)?;
                self.stat_key(&directory, path)
            }
            Err(other) => Err(other),
        }
    }

    fn stat_key(&self, key: &str, path: &Path) -> Result<opendal::Metadata> {
        let operator = self.operator.clone();
        let owned = key.to_string();
        dispatch(async move { operator.stat(&owned).await })
            .map_err(|error| self.failure("stat", path, error))
    }
}

/// `OpenDAL` metadata as the engine wants to see it.
///
/// No symlink case: no service models one, and the engine treats
/// `is_symlink: true` as "describe, never follow", which would be a lie here.
fn to_meta(metadata: &opendal::Metadata) -> Meta {
    Meta {
        len: metadata.content_length(),
        is_dir: metadata.is_dir(),
        is_symlink: false,
        modified: metadata.last_modified().map(Into::into),
    }
}

impl Backend for OpendalBackend {
    fn capabilities(&self) -> Capabilities {
        // Read, never probed. Writing a probe file into somebody's FTP root on
        // every open would be rude, and the local backend's probes
        // (`hard_link`, case folding) are filesystem calls with no protocol
        // analogue anyway.
        let capability = self.operator.info().capability();
        Capabilities {
            // OpenDAL reports "rename is supported", not "rename is atomic".
            // For every service registered in this slice the two coincide.
            // Slice 4c is where a service that renames by copy-then-delete
            // gets handled properly in `copy_verify_commit`.
            atomic_rename: capability.rename,
            // No service models hard links, and dedup must not be told it can
            // reclaim space it cannot reclaim.
            hard_links: false,
            // Unknowable without writing a probe file. `false` is the
            // conservative answer: treating a case-insensitive store as
            // sensitive is what silently overwrites `Holiday.mp4` with
            // `holiday.mp4`.
            case_sensitive: false,
        }
    }

    fn root_token(&self) -> Result<RootToken> {
        // The *connection*, not this link end's subfolder. A destination folder
        // nobody has created yet is an ordinary first write; a connection that
        // has gone away is the thing that must stop a drain.
        let unreachable = || BackendError::RootUnreachable(PathBuf::from(&self.endpoint));

        match &self.anchor {
            Anchor::LocalDir(root) => {
                let meta = std::fs::metadata(root).map_err(|_| unreachable())?;
                if !meta.is_dir() {
                    return Err(unreachable());
                }
            }
            Anchor::Store => {
                let operator = self.operator.clone();
                dispatch(async move { operator.list("/").await }).map_err(|_| unreachable())?;
            }
        }

        // No protocol exposes a filesystem device id, so this degrades to a
        // reachability check — exactly as it already does on Windows.
        Ok(RootToken { device: None })
    }

    fn stat(&self, path: &Path) -> Result<Meta> {
        self.stat_either(path).map(|m| to_meta(&m))
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<Entry>> {
        let key = self.directory(path)?;
        let operator = self.operator.clone();
        let owned = key.clone();
        let listed = dispatch(async move { operator.list(&owned).await })
            .map_err(|error| self.failure("list", path, error))?;

        let mut entries = Vec::with_capacity(listed.len());
        for entry in listed {
            let (raw, metadata) = entry.into_parts();
            let trimmed = raw.trim_start_matches('/').trim_end_matches('/');
            // A listing includes the directory being listed. Skipping it is
            // not cosmetic: leaving it in would make the walk recurse forever.
            if trimmed.is_empty() || raw.trim_end_matches('/') == key.trim_end_matches('/') {
                continue;
            }
            // Back to a path relative to the link end, so it can be fed
            // straight into any other method on this backend.
            let relative = strip_prefix(&self.prefix, trimmed);
            entries.push(Entry {
                path: PathBuf::from(relative),
                meta: to_meta(&metadata),
            });
        }
        Ok(entries)
    }

    fn open_read(&self, path: &Path) -> Result<Box<dyn Read + Send>> {
        let operator = self.operator.clone();
        let owned = self.key(path)?;
        let reader = dispatch(async move { operator.reader(&owned).await })
            .map_err(|error| self.failure("read", path, error))?;
        let len = self.stat_either(path)?.content_length();

        Ok(Box::new(OpendalRead {
            // `Reader::read` takes `&self`, so an `Arc` is all the sharing the
            // spawned future needs.
            reader: Arc::new(reader),
            offset: 0,
            len,
        }))
    }

    fn create_write(&self, path: &Path) -> Result<Box<dyn WriteFinish>> {
        let operator = self.operator.clone();
        let owned = self.key(path)?;
        let writer = dispatch(async move { operator.writer(&owned).await })
            .map_err(|error| self.failure("write", path, error))?;

        Ok(Box::new(OpendalWrite {
            writer: Some(writer),
            path: path.to_path_buf(),
            endpoint: self.endpoint.clone(),
        }))
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let (a, b) = (self.key(from)?, self.key(to)?);
        let operator = self.operator.clone();
        dispatch(async move { operator.rename(&a, &b).await })
            .map_err(|error| self.failure("rename", from, error))
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        let key = self.key(path)?;
        let operator = self.operator.clone();
        dispatch(async move { operator.delete(&key).await })
            .map_err(|error| self.failure("delete", path, error))
    }

    fn remove_dir(&self, path: &Path) -> Result<()> {
        let key = self.directory(path)?;
        let operator = self.operator.clone();
        dispatch(async move { operator.delete(&key).await })
            .map_err(|error| self.failure("delete", path, error))
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        // Same rail as the local backend: without this, a vanished root gets
        // recreated and the drain fills the disk it was emptying. OpenDAL's
        // `fs` builder is especially eager here — it creates its root at build
        // time — which is why the factory refuses a missing one up front.
        self.root_token()?;
        let key = self.directory(path)?;
        let operator = self.operator.clone();
        dispatch(async move { operator.create_dir(&key).await })
            .map_err(|error| self.failure("create_dir", path, error))
    }
}

/// A remote file being read, one engine-sized chunk per ranged request.
struct OpendalRead {
    reader: Arc<Reader>,
    offset: u64,
    len: u64,
}

impl Read for OpendalRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.offset >= self.len || buf.is_empty() {
            return Ok(0);
        }
        // Clamped to the known length: an open-ended range past the end is an
        // error on some services rather than a short read.
        let want = (buf.len() as u64).min(self.len - self.offset);
        let (start, end) = (self.offset, self.offset + want);
        let reader = Arc::clone(&self.reader);

        let buffer: Buffer = dispatch(async move { reader.read(start..end).await })
            .map_err(|error| std::io::Error::other(error.to_string()))?;

        // Clamped against a service that answers with more than it was asked
        // for. Indexing past `buf` would be a panic mid-drain.
        let read = buffer.len().min(buf.len());
        if read == 0 {
            // Zero bytes before the end is a truncated stream, not EOF. Saying
            // EOF here would hand the engine a short file whose destination
            // length matches what was sent, so verification would pass on a
            // file that is missing its tail.
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!(
                    "the remote stopped after {} of {} bytes",
                    self.offset, self.len
                ),
            ));
        }

        // `Buffer` is a rope of `Bytes`, so `to_vec` is the copy we would have
        // made into `buf` anyway rather than an extra one.
        buf[..read].copy_from_slice(&buffer.to_vec()[..read]);
        self.offset += read as u64;
        Ok(read)
    }
}

/// A remote file being written, committed by `finish`.
///
/// The writer is moved into each spawned future and moved back out, because
/// `Writer::write` takes `&mut self` and `spawn` needs an owned `'static`
/// future. `Option` is what makes the hand-off expressible.
struct OpendalWrite {
    writer: Option<Writer>,
    path: PathBuf,
    endpoint: String,
}

impl OpendalWrite {
    fn remote(&self, operation: &'static str, error: &opendal::Error) -> std::io::Error {
        std::io::Error::other(format!(
            "{operation} failed on `{}` at `{}`: {error}",
            self.endpoint,
            self.path.display()
        ))
    }
}

impl Write for OpendalWrite {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let Some(mut writer) = self.writer.take() else {
            return Err(std::io::Error::other("this writer has already been closed"));
        };
        let bytes = Buffer::from(buf.to_vec());

        let (writer, result) = dispatch(async move {
            let result = writer.write(bytes).await;
            (writer, result)
        });
        self.writer = Some(writer);

        result.map_err(|error| self.remote("write", &error))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // Nothing to do. OpenDAL buffers internally and commits on close, which
        // is what `finish` is for; a no-op here matches `File::flush`, which is
        // also a no-op against the page cache.
        Ok(())
    }
}

impl WriteFinish for OpendalWrite {
    fn finish(mut self: Box<Self>) -> Result<()> {
        let Some(mut writer) = self.writer.take() else {
            return Ok(());
        };
        let (writer, result) = dispatch(async move {
            let result = writer.close().await;
            (writer, result)
        });
        drop(writer);

        // The one that matters. A protocol write is not committed until the
        // stream is closed, so without this the journal could record a commit
        // for bytes the far side never accepted.
        result.map(|_| ()).map_err(|error| BackendError::Remote {
            endpoint: self.endpoint.clone(),
            operation: "close",
            source: Box::new(error),
        })
    }
}
