//! A [`Backend`] over SMB, reached directly rather than through a mount.
//!
//! Built on smb-rs's synchronous client, so there is no runtime to own and no
//! future to block on: each [`Backend`] method is a handful of SMB requests
//! made on the calling thread. One client serves every file of a run; SMB
//! multiplexes requests over one connection, so the transfer engine's workers
//! share it rather than each signing in.
//!
//! The connection's root is the share followed by the folder inside it. That
//! folder is the anchor: if it cannot be seen, the share is gone and the run
//! must stop, which is the same promise the local backend makes about a
//! vanished mount.

mod path;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use smb::connection::EncryptionMode;
use smb::{
    Client, ClientConfig, ConnectionConfig, CreateDisposition, CreateOptions, DirAccessMask,
    Directory, FileAccessMask, FileAttributes, FileBasicInformation, FileCreateArgs,
    FileDispositionInformation, FileFullDirectoryInformation, FileNetworkOpenInformation,
    FileRenameInformation, ReadAt, Resource, UncPath, WriteAt,
};
use tungstate_backend::{
    Backend, BackendError, Capabilities, Entry, Meta, Result, RootToken, WriteFinish,
};

pub use path::split_root;

/// Where a share is and how to sign in to it.
#[derive(Clone)]
pub struct Settings {
    /// The server's name or address.
    pub host: String,
    /// Where it listens, when not on 445.
    pub port: Option<u16>,
    /// What the connection's root says: the share, then the folder inside.
    pub root: String,
    /// Who to sign in as.
    pub username: String,
    /// Their password. SMB never sends it as it is.
    pub password: String,
    /// Refuse a server that will not encrypt, rather than leave it to the
    /// server to decide.
    pub require_encryption: bool,
}

// By hand, so the password never reaches a log line.
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("root", &self.root)
            .field("username", &self.username)
            .field("require_encryption", &self.require_encryption)
            .finish_non_exhaustive()
    }
}

/// NT status codes this backend reads. Protocol constants from MS-ERREF, the
/// same on every server.
mod status {
    pub const NO_SUCH_FILE: u32 = 0xC000_000F;
    pub const OBJECT_NAME_NOT_FOUND: u32 = 0xC000_0034;
    pub const OBJECT_PATH_NOT_FOUND: u32 = 0xC000_003A;
    pub const LOGON_FAILURE: u32 = 0xC000_006D;
    pub const BAD_NETWORK_NAME: u32 = 0xC000_00CC;
}

/// How long one request may take before the share counts as gone.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The most one read or write asks for, whatever the server offers.
///
/// Samba offers 8 MiB, and with smb-rs 0.12 a request that size is answered
/// and then the next one waits for ever, past the timeout. Found by testing
/// against Samba: every size up to 4 MiB was fine and no faster than 1 MiB,
/// which is also the most the engine hands over at once.
const MOST_PER_REQUEST: usize = 1024 * 1024;

/// A [`Backend`] over one folder inside one SMB share.
pub struct SmbBackend {
    client: Arc<Client>,
    /// `\\host\share`, which every name below is inside.
    share: UncPath,
    /// The connection's folder inside the share: the anchor.
    anchor: Vec<String>,
    /// The anchor followed by this link end's own folder.
    base: Vec<String>,
    /// The connection name, for error messages.
    endpoint: String,
    /// The most one read or write request may carry: what the server said when
    /// the connection was made, and never more than [`MOST_PER_REQUEST`].
    max_read: usize,
    max_write: usize,
}

impl std::fmt::Debug for SmbBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmbBackend")
            .field("share", &self.share.to_string())
            .field("base", &self.base)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl SmbBackend {
    /// Sign in to the share and point at `link_end` inside the connection's
    /// folder. `endpoint` is the connection's name, for error messages.
    ///
    /// The link end need not exist yet; the connection's folder must.
    ///
    /// # Errors
    /// [`BackendError::Auth`] if the server refuses the user or password,
    /// [`BackendError::RootUnreachable`] if the server, the share or the
    /// folder cannot be reached, and the path errors for a link end that
    /// would leave the folder.
    pub fn connect(settings: &Settings, link_end: &Path, endpoint: &str) -> Result<Self> {
        let unreachable = || BackendError::RootUnreachable(PathBuf::from(endpoint));
        let (share_name, anchor) = split_root(&settings.root).ok_or_else(unreachable)?;
        let mut base = anchor.clone();
        base.extend(path::parts(link_end)?);

        let mut connection = ConnectionConfig {
            port: settings.port,
            timeout: Some(TIMEOUT),
            encryption_mode: if settings.require_encryption {
                EncryptionMode::Required
            } else {
                EncryptionMode::Allowed
            },
            ..ConnectionConfig::default()
        };
        // A home NAS signs in with a user and a password; Kerberos would go
        // looking for a domain that is not there.
        connection.auth_methods.kerberos = false;
        connection.auth_methods.ntlm = true;
        let client = Client::new(ClientConfig {
            connection,
            ..ClientConfig::default()
        });

        let share = UncPath::from_str(&format!(r"\\{}\{share_name}", settings.host))
            .map_err(|_| unreachable())?;
        client
            .share_connect(&share, &settings.username, settings.password.clone())
            .map_err(|error| {
                if status_of(&error) == Some(status::LOGON_FAILURE) {
                    return BackendError::Auth {
                        endpoint: endpoint.to_string(),
                    };
                }
                tracing::debug!(%error, endpoint, "could not reach the share");
                remote(endpoint, "connect", error)
            })?;

        // 64 KiB is what every SMB 2 server accepts, and only matters if the
        // server somehow did not say.
        let (max_read, max_write) = client
            .get_connection(&settings.host)
            .ok()
            .and_then(|connection| {
                connection.conn_info().map(|info| {
                    let n = &info.negotiation;
                    (n.max_read_size as usize, n.max_write_size as usize)
                })
            })
            .unwrap_or((65_536, 65_536));

        let backend = Self {
            client: Arc::new(client),
            share,
            anchor,
            base,
            endpoint: endpoint.to_string(),
            max_read: max_read.clamp(4096, MOST_PER_REQUEST),
            max_write: max_write.clamp(4096, MOST_PER_REQUEST),
        };
        backend.root_token()?;
        Ok(backend)
    }

    /// The share-relative name of `relative` inside this link end.
    fn name(&self, relative: &Path) -> Result<String> {
        let mut parts = self.base.clone();
        parts.extend(path::parts(relative)?);
        Ok(path::join(&parts))
    }

    fn unc(&self, name: &str) -> UncPath {
        self.share.clone().with_path(name)
    }

    fn open(&self, name: &str, args: &FileCreateArgs) -> smb::Result<Resource> {
        self.client.create_file(&self.unc(name), args)
    }

    /// A file opened for reading, from the start.
    fn reader(&self, path: &Path) -> Result<SmbRead> {
        let name = self.name(path)?;
        let access = FileAccessMask::new()
            .with_generic_read(true)
            .with_synchronize(true);
        let resource = self
            .open(&name, &FileCreateArgs::make_open_existing(access))
            .map_err(|error| self.failure("read", path, error))?;
        let Resource::File(file) = resource else {
            return Err(BackendError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::other("not a file"),
            });
        };
        Ok(SmbRead {
            file: Some(file),
            offset: 0,
            most: self.max_read,
        })
    }

    /// A failure as the engine reads it: a missing file is an ordinary
    /// not-found, which it takes to mean "nothing is there".
    fn failure(&self, operation: &'static str, path: &Path, error: smb::Error) -> BackendError {
        match status_of(&error) {
            Some(
                status::NO_SUCH_FILE
                | status::OBJECT_NAME_NOT_FOUND
                | status::OBJECT_PATH_NOT_FOUND,
            ) => BackendError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, error.to_string()),
            },
            _ => remote(&self.endpoint, operation, error),
        }
    }

    /// Metadata for a name, without reading the file.
    fn stat_name(&self, name: &str, path: &Path) -> Result<Meta> {
        let access = FileAccessMask::new()
            .with_file_read_attributes(true)
            .with_synchronize(true);
        let resource = self
            .open(name, &FileCreateArgs::make_open_existing(access))
            .map_err(|error| self.failure("stat", path, error))?;
        let (info, closed) = match &resource {
            Resource::File(file) => (
                file.query_info::<FileNetworkOpenInformation>(),
                file.close(),
            ),
            Resource::Directory(dir) => {
                (dir.query_info::<FileNetworkOpenInformation>(), dir.close())
            }
            Resource::Pipe(_) => {
                return Err(BackendError::Io {
                    path: path.to_path_buf(),
                    source: std::io::Error::other("a pipe, not a file or folder"),
                });
            }
        };
        let info = info.map_err(|error| self.failure("stat", path, error))?;
        if let Err(error) = closed {
            tracing::debug!(%error, "closing after a stat failed");
        }
        Ok(meta(
            info.end_of_file,
            info.file_attributes,
            info.last_write_time.into(),
        ))
    }

    /// Open a name with the right to delete it, set `info`, and close. The
    /// change takes effect at the close, which is why it is waited for.
    fn change<T>(&self, name: &str, path: &Path, operation: &'static str, info: T) -> Result<()>
    where
        T: smb::SetFileInfoValue,
    {
        let access = FileAccessMask::new()
            .with_delete(true)
            .with_file_read_attributes(true)
            .with_synchronize(true);
        let resource = self
            .open(name, &FileCreateArgs::make_open_existing(access))
            .map_err(|error| self.failure(operation, path, error))?;
        let result = match &resource {
            Resource::File(file) => file.set_info(info).and_then(|()| file.close()),
            Resource::Directory(dir) => dir.set_info(info).and_then(|()| dir.close()),
            Resource::Pipe(_) => Ok(()),
        };
        result.map_err(|error| self.failure(operation, path, error))
    }

    /// Make one folder, or find it already there.
    fn make_folder(&self, name: &str, path: &Path) -> Result<()> {
        let args = FileCreateArgs {
            disposition: CreateDisposition::OpenIf,
            attributes: FileAttributes::new().with_directory(true),
            options: CreateOptions::new().with_directory_file(true),
            desired_access: FileAccessMask::new()
                .with_file_read_attributes(true)
                .with_synchronize(true),
        };
        let resource = self
            .open(name, &args)
            .map_err(|error| self.failure("create_dir", path, error))?;
        match resource {
            Resource::Directory(dir) => dir
                .close()
                .map_err(|error| self.failure("create_dir", path, error)),
            // A file already holds the name. The engine reports it; SMB would
            // otherwise have quietly opened the file as if it were the folder.
            Resource::File(file) => {
                let _ = file.close();
                Err(BackendError::Io {
                    path: path.to_path_buf(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        "a file already has this folder's name",
                    ),
                })
            }
            Resource::Pipe(_) => Ok(()),
        }
    }
}

impl Drop for SmbBackend {
    fn drop(&mut self) {
        // Signing out is a courtesy to the server, and the process may be
        // ending anyway; nothing depends on it succeeding.
        if let Err(error) = self.client.close() {
            tracing::debug!(%error, "closing an SMB client");
        }
    }
}

/// The NT status a failure carries, if it carries one.
fn status_of(error: &smb::Error) -> Option<u32> {
    match error {
        smb::Error::ReceivedErrorMessage(status, _)
        | smb::Error::UnexpectedMessageStatus(status) => Some(*status),
        _ => None,
    }
}

fn remote(endpoint: &str, operation: &'static str, error: smb::Error) -> BackendError {
    if status_of(&error) == Some(status::BAD_NETWORK_NAME) {
        return BackendError::RootUnreachable(PathBuf::from(endpoint));
    }
    BackendError::Remote {
        endpoint: endpoint.to_string(),
        operation,
        source: Box::new(error),
    }
}

/// A time as SMB writes it: tenths of a microsecond since 1601, in UTC.
fn windows_ticks(at: SystemTime) -> Option<u64> {
    /// Seconds from 1601 to 1970, the two epochs.
    const EPOCHS_APART: u64 = 11_644_473_600;
    let since = at.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    let ticks = u128::from(since.as_secs() + EPOCHS_APART) * 10_000_000
        + u128::from(since.subsec_nanos() / 100);
    u64::try_from(ticks).ok()
}

fn meta(len: u64, attributes: FileAttributes, modified: SystemTime) -> Meta {
    Meta {
        len,
        is_dir: attributes.directory(),
        // A reparse point is SMB's symlink. Described, never followed.
        is_symlink: attributes.reparse_point(),
        modified: Some(modified),
        // SMB has file ids, but nothing here trusts one across servers yet,
        // and a wrong one turns two copies into "one file under two names".
        identity: None,
    }
}

impl Backend for SmbBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            // SMB renames within a share in one request, on the server.
            atomic_rename: true,
            // No protocol call makes one, and dedup must not be told it can.
            hard_links: false,
            // Windows shares and Samba's defaults both ignore case. `false` is
            // the answer that never overwrites `Holiday.mp4` with `holiday.mp4`.
            case_sensitive: false,
            networked: true,
        }
    }

    fn root_token(&self) -> Result<RootToken> {
        // The connection's folder, not this link end's: a destination folder
        // nobody has made yet is an ordinary first write, and a share that
        // has gone is what must stop a run.
        let anchor = path::join(&self.anchor);
        match self.stat_name(&anchor, Path::new("")) {
            Ok(meta) if meta.is_dir => Ok(RootToken { device: None }),
            _ => Err(BackendError::RootUnreachable(PathBuf::from(&self.endpoint))),
        }
    }

    fn stat(&self, path: &Path) -> Result<Meta> {
        let name = self.name(path)?;
        self.stat_name(&name, path)
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<Entry>> {
        let name = self.name(path)?;
        let access = DirAccessMask::new()
            .with_list_directory(true)
            .with_synchronize(true);
        let resource = match self.open(&name, &FileCreateArgs::make_open_existing(access.into())) {
            Ok(resource) => resource,
            Err(error) => {
                let failure = self.failure("list", path, error);
                // This link end's own folder, not made yet, is empty rather
                // than missing: the same first write `root_token` allows, and
                // how the FTP and S3 services answer. A share that has gone
                // still fails, because the anchor is asked again.
                if path.as_os_str().is_empty()
                    && matches!(&failure, BackendError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound)
                {
                    self.root_token()?;
                    return Ok(Vec::new());
                }
                return Err(failure);
            }
        };
        let Resource::Directory(dir) = resource else {
            return Err(BackendError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::other("not a folder"),
            });
        };

        let mut entries = Vec::new();
        let listed = Directory::query::<FileFullDirectoryInformation>(&dir, "*")
            .map_err(|error| self.failure("list", path, error))
            .and_then(|found| {
                for item in found {
                    let item = item.map_err(|error| self.failure("list", path, error))?;
                    let file_name = item.file_name.to_string();
                    if file_name == "." || file_name == ".." {
                        continue;
                    }
                    entries.push(Entry {
                        path: path.join(&file_name),
                        meta: meta(
                            item.end_of_file,
                            item.file_attributes,
                            item.last_write_time.into(),
                        ),
                    });
                }
                Ok(())
            });
        let closed = dir.close();
        listed?;
        closed.map_err(|error| self.failure("list", path, error))?;
        Ok(entries)
    }

    fn open_read(&self, path: &Path) -> Result<Box<dyn Read + Send>> {
        Ok(Box::new(self.reader(path)?))
    }

    fn read_prefix(&self, path: &Path, len: u64) -> Result<Vec<u8>> {
        self.read_range(path, 0, len)
    }

    fn read_range(&self, path: &Path, offset: u64, len: u64) -> Result<Vec<u8>> {
        let mut reader = self.reader(path)?;
        // Every SMB read is a request at an offset, never a stream, so
        // starting the reader further in is all a ranged read needs.
        reader.offset = offset;
        let mut bytes = Vec::new();
        reader
            .take(len)
            .read_to_end(&mut bytes)
            .map_err(|source| BackendError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(bytes)
    }

    fn set_modified(&self, path: &Path, at: SystemTime) -> Result<bool> {
        // Before 1970 is a time no file being synced has; say it cannot be set.
        let Some(ticks) = windows_ticks(at) else {
            return Ok(false);
        };
        let name = self.name(path)?;
        let access = FileAccessMask::new()
            .with_file_write_attributes(true)
            .with_synchronize(true);
        let resource = self
            .open(&name, &FileCreateArgs::make_open_existing(access))
            .map_err(|error| self.failure("set_modified", path, error))?;
        // Zero means "leave it" for every time but the one being set.
        let info = FileBasicInformation {
            creation_time: 0.into(),
            last_access_time: 0.into(),
            last_write_time: ticks.into(),
            change_time: 0.into(),
            file_attributes: FileAttributes::new(),
        };
        let result = match &resource {
            Resource::File(file) => file.set_info(info).and_then(|()| file.close()),
            Resource::Directory(dir) => dir.set_info(info).and_then(|()| dir.close()),
            Resource::Pipe(_) => Ok(()),
        };
        result
            .map(|()| true)
            .map_err(|error| self.failure("set_modified", path, error))
    }

    fn create_write(&self, path: &Path) -> Result<Box<dyn WriteFinish>> {
        let name = self.name(path)?;
        let resource = self
            .open(
                &name,
                &FileCreateArgs::make_overwrite(FileAttributes::new(), CreateOptions::new()),
            )
            .map_err(|error| self.failure("write", path, error))?;
        let Resource::File(file) = resource else {
            return Err(BackendError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::other("a folder has this file's name"),
            });
        };
        Ok(Box::new(SmbWrite {
            file: Some(file),
            offset: 0,
            most: self.max_write,
            path: path.to_path_buf(),
            endpoint: self.endpoint.clone(),
        }))
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let (source, target) = (self.name(from)?, self.name(to)?);
        let info = FileRenameInformation {
            replace_if_exists: true.into(),
            root_directory: 0,
            file_name: target.as_str().into(),
        };
        self.change(&source, from, "rename", info)
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        let name = self.name(path)?;
        self.change(&name, path, "delete", FileDispositionInformation::default())
    }

    fn remove_dir(&self, path: &Path) -> Result<()> {
        // The server refuses a folder that is not empty, which is the contract.
        self.remove_file(path)
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        // The same rail as the local backend: never build a folder chain on a
        // share that has gone, or recreate a folder somebody removed.
        self.root_token()?;
        let mut parts = self.base.clone();
        let mut made = self.anchor.len();
        parts.extend(path::parts(path)?);
        // The anchor exists, as `root_token` just proved. Everything after it
        // is made in turn, each found or created.
        while made < parts.len() {
            made += 1;
            self.make_folder(&path::join(&parts[..made]), path)?;
        }
        Ok(())
    }
}

/// A file being read, a request at a time.
struct SmbRead {
    /// `None` once closed.
    file: Option<smb::File>,
    offset: u64,
    /// The most one request may ask for.
    most: usize,
}

impl Read for SmbRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(file) = &self.file else {
            return Ok(0);
        };
        let take = buf.len().min(self.most);
        let got = file
            .read_at(&mut buf[..take], self.offset)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        self.offset += got as u64;
        if got == 0
            && let Some(file) = self.file.take()
        {
            let _ = file.close();
        }
        Ok(got)
    }
}

impl Drop for SmbRead {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = file.close();
        }
    }
}

/// A file being written, committed by `finish`.
struct SmbWrite {
    file: Option<smb::File>,
    offset: u64,
    most: usize,
    path: PathBuf,
    endpoint: String,
}

impl Write for SmbWrite {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let Some(file) = &self.file else {
            return Err(std::io::Error::other("this writer has already been closed"));
        };
        let take = buf.len().min(self.most);
        let wrote = file
            .write_at(&buf[..take], self.offset)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        self.offset += wrote as u64;
        Ok(wrote)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // What makes it durable is `finish`, as for a local file.
        Ok(())
    }
}

impl WriteFinish for SmbWrite {
    fn finish(mut self: Box<Self>) -> Result<()> {
        let Some(file) = self.file.take() else {
            return Ok(());
        };
        // Flush asks the server to put the bytes on its disk, and the close
        // is when the file is complete for anyone else. Both have to succeed
        // before the journal may say this file arrived.
        let flushed = file.flush().map_err(smb::Error::from);
        let closed = file.close();
        flushed.and(closed).map_err(|error| BackendError::Remote {
            endpoint: self.endpoint.clone(),
            operation: "close",
            source: Box::new(std::io::Error::other(format!(
                "`{}` did not finish: {error}",
                self.path.display()
            ))),
        })
    }
}

impl Drop for SmbWrite {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = file.close();
        }
    }
}

#[cfg(test)]
mod tests;
