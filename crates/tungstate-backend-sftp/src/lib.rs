//! A [`Backend`] over SFTP, on `russh` and `russh-sftp`.
//!
//! Hand-rolled rather than `OpenDAL`'s SFTP service, which is Unix-only,
//! refuses passwords, and leaves the server's identity to whatever the
//! system's `ssh` is configured with (DESIGN.md §6). Here the identity is
//! the connection's own: the first time a server is reached its key is shown
//! to the person and kept with the connection once they trust it, and a
//! server that later presents a different key is refused.
//!
//! One SSH connection serves every file of a run. SFTP numbers its requests,
//! so the engine's workers share it, and a large read or write keeps many
//! requests in flight at once rather than waiting for each in turn.

mod path;
mod runtime;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::future::join_all;
use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey};
use russh::{MethodKind, client::KeyboardInteractiveAuthResponse};
use russh_sftp::client::RawSftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::extensions::HardlinkExtension;
use russh_sftp::protocol::{FileAttributes, OpenFlags, Packet, StatusCode};
use tungstate_backend::{
    Backend, BackendError, Capabilities, Entry, Meta, Result, RootToken, WriteFinish,
};

use crate::runtime::dispatch;

/// Where a server is and how to sign in to it.
#[derive(Clone)]
pub struct Settings {
    /// The server's name or address.
    pub host: String,
    /// Where it listens, when not on 22.
    pub port: Option<u16>,
    /// The folder every path is inside: absolute, or relative to where the
    /// account lands. Empty means where it lands.
    pub root: String,
    /// Who to sign in as.
    pub username: String,
    /// How.
    pub auth: Auth,
    /// The server's key as trusted, in the one-line form `ssh` writes.
    /// `None` until the person has trusted one.
    pub host_key: Option<String>,
}

/// How to prove who we are.
#[derive(Clone)]
pub enum Auth {
    /// A password, offered as `password` and, where the server prefers it,
    /// as `keyboard-interactive`, which several NAS boxes use instead.
    Password(String),
    /// A private key file, and its passphrase if it has one.
    Key {
        /// Where the key is on this machine.
        path: PathBuf,
        /// What unlocks it.
        passphrase: Option<String>,
    },
}

// By hand, so a password or passphrase never reaches a log line.
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let auth = match &self.auth {
            Auth::Password(_) => "password".to_string(),
            Auth::Key { path, .. } => format!("key {}", path.display()),
        };
        f.debug_struct("Settings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("root", &self.root)
            .field("username", &self.username)
            .field("auth", &auth)
            .field("host_key", &self.host_key.is_some())
            .finish()
    }
}

/// How much one read or write request carries. 32 KiB is what every SFTP
/// server must accept; OpenSSH takes more, but no faster once requests are
/// kept in flight.
const PIECE_LEN: u32 = 32 * 1024;
/// The same, for slicing buffers.
const PIECE: usize = PIECE_LEN as usize;

/// Requests kept in flight by one read: 1 MiB at a time, the most the engine
/// hands over at once.
const PIECES_AT_ONCE: usize = 32;

/// How long one request may take. Long, because a NAS spinning its disks up
/// can take most of that before it answers anything.
const REQUEST_TIMEOUT_SECS: u64 = 60;

/// Watches the server's key during the handshake.
struct Guard {
    trusted: Option<PublicKey>,
    /// What the server presented, kept so a refusal can say what it was.
    seen: Arc<Mutex<Option<PublicKey>>>,
}

impl client::Handler for Guard {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        server: &PublicKey,
    ) -> impl Future<Output = std::result::Result<bool, Self::Error>> + Send {
        let key = server.clone();
        let matches = self
            .trusted
            .as_ref()
            .is_some_and(|trusted| trusted.key_data() == key.key_data());
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(key);
        // Decided here, before the future: there is nothing to wait for.
        std::future::ready(Ok(matches))
    }
}

/// One signed-in SSH connection and the SFTP session on it.
struct Live {
    ssh: Handle<Guard>,
    sftp: RawSftpSession,
    /// The server can rename over an existing file in one step.
    posix_rename: bool,
    /// The server can be asked to put a file's bytes on its disk.
    fsync: bool,
}

/// A [`Backend`] over one folder on an SFTP server.
pub struct SftpBackend {
    /// Kept to dial again: `russh` never reconnects by itself.
    settings: Settings,
    endpoint: String,
    /// The connection's folder: if it cannot be seen, the run stops.
    anchor: String,
    /// This link end's own folder inside it, part by part.
    inside: Vec<String>,
    /// The two together, which every path is under.
    base: String,
    live: Mutex<Arc<Live>>,
    /// Decided once, at the first sign-in, so capabilities never change
    /// under a run.
    posix_rename: bool,
}

impl std::fmt::Debug for SftpBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SftpBackend")
            .field("host", &self.settings.host)
            .field("base", &self.base)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl SftpBackend {
    /// Sign in and point at `link_end` inside the connection's folder.
    /// `endpoint` is the connection's name, for messages.
    ///
    /// The link end need not exist yet; the connection's folder must.
    ///
    /// # Errors
    /// [`BackendError::HostUnknown`] the first time a server is reached,
    /// with the key to show and keep; [`BackendError::HostKeyChanged`] if it
    /// is not the server trusted; [`BackendError::Auth`] if the sign-in is
    /// refused; [`BackendError::RootUnreachable`] if the folder is not there.
    pub fn connect(settings: &Settings, link_end: &Path, endpoint: &str) -> Result<Self> {
        let anchor = path::root(&settings.root);
        let inside = path::parts(link_end)?;
        let base = path::under(&anchor, &inside);
        let live = dispatch(dial(settings.clone(), endpoint.to_string()))?;
        let backend = Self {
            settings: settings.clone(),
            endpoint: endpoint.to_string(),
            anchor,
            inside,
            base,
            posix_rename: live.posix_rename,
            live: Mutex::new(Arc::new(live)),
        };
        backend.root_token()?;
        Ok(backend)
    }

    /// The session, dialled again if the connection has dropped.
    fn live(&self) -> Result<Arc<Live>> {
        let mut held = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        if held.ssh.is_closed() {
            tracing::debug!(endpoint = %self.endpoint, "the SFTP connection dropped; dialling again");
            *held = Arc::new(dispatch(dial(
                self.settings.clone(),
                self.endpoint.clone(),
            ))?);
        }
        Ok(Arc::clone(&held))
    }

    /// The server's path for `relative` inside this link end.
    fn name(&self, relative: &Path) -> Result<String> {
        Ok(path::under(&self.base, &path::parts(relative)?))
    }

    /// Run one request against the session and map its failure.
    fn call<T, F, Fut>(&self, operation: &'static str, path: &Path, request: F) -> Result<T>
    where
        F: FnOnce(Arc<Live>) -> Fut,
        Fut: Future<Output = std::result::Result<T, SftpError>> + Send + 'static,
        T: Send + 'static,
    {
        let live = self.live()?;
        dispatch(request(live)).map_err(|error| self.failure(operation, path, &error))
    }

    /// A failure as the engine reads it: a missing file is an ordinary
    /// not-found, which it takes to mean "nothing is there".
    fn failure(&self, operation: &'static str, path: &Path, error: &SftpError) -> BackendError {
        if is_missing(error) {
            return BackendError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, error.to_string()),
            };
        }
        BackendError::Remote {
            endpoint: self.endpoint.clone(),
            operation,
            source: Box::new(std::io::Error::other(error.to_string())),
        }
    }

    fn lstat(&self, path: &Path) -> Result<FileAttributes> {
        let name = self.name(path)?;
        self.call("stat", path, move |live| async move {
            live.sftp.lstat(name).await.map(|found| found.attrs)
        })
    }

    /// A file opened for reading, from the start.
    fn reader(&self, path: &Path) -> Result<SftpRead> {
        let name = self.name(path)?;
        let live = self.live()?;
        let session = Arc::clone(&live);
        let handle = dispatch(async move {
            session
                .sftp
                .open(name, OpenFlags::READ, FileAttributes::default())
                .await
                .map(|opened| opened.handle)
        })
        .map_err(|error| self.failure("read", path, &error))?;
        Ok(SftpRead {
            live,
            handle: Some(handle),
            offset: 0,
            ready: Vec::new(),
            taken: 0,
            done: false,
        })
    }

    /// Make one folder, or find it already there.
    fn make_folder(&self, name: String, path: &Path) -> Result<()> {
        self.call("create_dir", path, move |live| async move {
            if let Ok(found) = live.sftp.lstat(name.clone()).await
                && found.attrs.is_dir()
            {
                return Ok(());
            }
            match live
                .sftp
                .mkdir(name.clone(), FileAttributes::default())
                .await
            {
                Ok(_) => Ok(()),
                // SFTP says only "Failure" when the folder exists, which is
                // what another worker making the same folder leaves behind.
                // Looking again tells that apart from a real refusal.
                Err(error) => match live.sftp.lstat(name).await {
                    Ok(found) if found.attrs.is_dir() => Ok(()),
                    _ => Err(error),
                },
            }
        })
    }
}

/// Sign in, check the server is the one trusted, and start SFTP.
async fn dial(settings: Settings, endpoint: String) -> Result<Live> {
    let trusted = settings
        .host_key
        .as_deref()
        .map(|line| PublicKey::from_openssh(line.trim()))
        .transpose()
        .map_err(|error| BackendError::Remote {
            endpoint: endpoint.clone(),
            operation: "connect",
            source: Box::new(std::io::Error::other(format!(
                "the trusted server key could not be read: {error}"
            ))),
        })?;
    let seen = Arc::new(Mutex::new(None));
    let config = Arc::new(client::Config {
        // A larger window than the default 2 MiB lets a whole engine chunk
        // be in flight before the server has to say "more".
        window_size: 8 * 1024 * 1024,
        nodelay: true,
        keepalive_interval: Some(Duration::from_secs(20)),
        keepalive_max: 3,
        ..client::Config::default()
    });
    let port = settings.port.unwrap_or(22);

    // OpenSSH drops new connections at random once ten are mid-handshake
    // (`MaxStartups`), and a NAS keeps that default, so a run whose workers
    // all dial at once can lose one. Dropped before a key was even shown is
    // that, or the network, and worth two more tries; anything after is not.
    let mut tries = 0;
    let mut ssh = loop {
        tries += 1;
        let guard = Guard {
            trusted: trusted.clone(),
            seen: Arc::clone(&seen),
        };
        match client::connect(Arc::clone(&config), (settings.host.as_str(), port), guard).await {
            Ok(ssh) => break ssh,
            Err(error) => {
                let presented = seen.lock().unwrap_or_else(PoisonError::into_inner).take();
                match (presented, &trusted) {
                    (Some(key), None) => return Err(unknown(&endpoint, &key, false)),
                    (Some(key), Some(trusted)) if key.key_data() != trusted.key_data() => {
                        return Err(unknown(&endpoint, &key, true));
                    }
                    (None, _) if tries < 3 => {
                        tokio::time::sleep(Duration::from_millis(250 * tries)).await;
                    }
                    _ => return Err(remote(&endpoint, "connect", &error)),
                }
            }
        }
    };

    let user = settings.username.clone();
    let signed_in = match &settings.auth {
        Auth::Password(password) => sign_in_with_password(&mut ssh, &user, password).await,
        Auth::Key { path, passphrase } => {
            let key =
                russh::keys::load_secret_key(path, passphrase.as_deref()).map_err(|error| {
                    BackendError::Remote {
                        endpoint: endpoint.clone(),
                        operation: "connect",
                        source: Box::new(std::io::Error::other(format!(
                            "the key at {} could not be opened: {error}",
                            path.display()
                        ))),
                    }
                })?;
            // RSA keys sign with SHA-2 where the server allows it, rather
            // than the SHA-1 servers have stopped accepting.
            let hash = ssh.best_supported_rsa_hash().await.ok().flatten().flatten();
            ssh.authenticate_publickey(&user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map(|answer| answer.success())
        }
    }
    .map_err(|error| remote(&endpoint, "sign in", &error))?;
    if !signed_in {
        return Err(BackendError::Auth { endpoint });
    }

    let channel = ssh
        .channel_open_session()
        .await
        .map_err(|error| remote(&endpoint, "connect", &error))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| remote(&endpoint, "connect", &error))?;
    let sftp = RawSftpSession::new_with_config(
        channel.into_stream(),
        russh_sftp::client::Config {
            request_timeout_secs: REQUEST_TIMEOUT_SECS,
            ..russh_sftp::client::Config::default()
        },
    );
    let version = sftp
        .init()
        .await
        .map_err(|error| remote(&endpoint, "connect", &error))?;
    let offers = |name: &str| version.extensions.get(name).is_some_and(|v| v == "1");
    Ok(Live {
        posix_rename: offers("posix-rename@openssh.com"),
        fsync: offers("fsync@openssh.com"),
        ssh,
        sftp,
    })
}

/// A password, then `keyboard-interactive` with the same password where the
/// server would rather ask that way.
async fn sign_in_with_password(
    ssh: &mut Handle<Guard>,
    user: &str,
    password: &str,
) -> std::result::Result<bool, russh::Error> {
    let first = ssh.authenticate_password(user, password).await?;
    let russh::client::AuthResult::Failure {
        remaining_methods, ..
    } = first
    else {
        return Ok(true);
    };
    if !remaining_methods.contains(&MethodKind::KeyboardInteractive) {
        return Ok(false);
    }
    let mut answer = ssh
        .authenticate_keyboard_interactive_start(user, None::<String>)
        .await?;
    // A few rounds at most: a server that keeps asking is not asking for a
    // password, and answering it for ever would hang the run.
    for _ in 0..4 {
        match answer {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let replies = prompts.iter().map(|_| password.to_string()).collect();
                answer = ssh
                    .authenticate_keyboard_interactive_respond(replies)
                    .await?;
            }
        }
    }
    Ok(false)
}

/// The server's key, as the person is shown it and as it is kept.
fn unknown(endpoint: &str, key: &PublicKey, changed: bool) -> BackendError {
    let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
    let key = key.to_openssh().unwrap_or_default();
    let endpoint = endpoint.to_string();
    if changed {
        BackendError::HostKeyChanged {
            endpoint,
            fingerprint,
            key,
        }
    } else {
        BackendError::HostUnknown {
            endpoint,
            fingerprint,
            key,
        }
    }
}

fn remote(endpoint: &str, operation: &'static str, error: &dyn std::fmt::Display) -> BackendError {
    BackendError::Remote {
        endpoint: endpoint.to_string(),
        operation,
        source: Box::new(std::io::Error::other(error.to_string())),
    }
}

fn is_missing(error: &SftpError) -> bool {
    matches!(error, SftpError::Status(status) if status.status_code == StatusCode::NoSuchFile)
}

/// What the engine knows about an entry, from SFTP's attributes.
fn meta(attrs: &FileAttributes) -> Meta {
    Meta {
        len: attrs.size.unwrap_or(0),
        is_dir: attrs.is_dir(),
        // Described, never followed, as everywhere else.
        is_symlink: attrs.is_symlink(),
        // SFTP version 3 carries whole seconds.
        modified: attrs
            .mtime
            .map(|seconds| UNIX_EPOCH + Duration::from_secs(u64::from(seconds))),
        identity: None,
        online_only: false,
    }
}

impl Backend for SftpBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            // Only with OpenSSH's `posix-rename`: plain SFTP rename refuses a
            // target that exists, which is the one rename the engine needs.
            atomic_rename: self.posix_rename,
            hard_links: false,
            // Most NAS volumes are case-sensitive, but `false` is the answer
            // that never overwrites `Holiday.mp4` with `holiday.mp4`.
            case_sensitive: false,
            networked: true,
        }
    }

    fn root_token(&self) -> Result<RootToken> {
        let anchor = self.anchor.clone();
        // `stat`, not `lstat`: a connection folder that is a link to the
        // real one, common on a NAS, is still there.
        let found = self.call("stat", Path::new(""), move |live| async move {
            live.sftp.stat(anchor).await.map(|found| found.attrs)
        });
        match found {
            Ok(attrs) if attrs.is_dir() => Ok(RootToken { device: None }),
            _ => Err(BackendError::RootUnreachable(PathBuf::from(&self.endpoint))),
        }
    }

    fn stat(&self, path: &Path) -> Result<Meta> {
        self.lstat(path).map(|attrs| meta(&attrs))
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<Entry>> {
        let name = self.name(path)?;
        let listed = self.call("list", path, move |live| async move {
            let handle = live.sftp.opendir(name).await?.handle;
            let mut found = Vec::new();
            let listed = loop {
                match live.sftp.readdir(handle.clone()).await {
                    Ok(page) => found.extend(page.files),
                    Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                        break Ok(found);
                    }
                    Err(error) => break Err(error),
                }
            };
            let _ = live.sftp.close(handle).await;
            listed
        });
        let files = match listed {
            Ok(files) => files,
            // This link end's own folder, not made yet, is empty rather than
            // missing, as over SMB, FTP and S3. A vanished connection folder
            // still fails, because the anchor is asked again.
            Err(BackendError::Io { source, .. })
                if path.as_os_str().is_empty() && source.kind() == std::io::ErrorKind::NotFound =>
            {
                self.root_token()?;
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        Ok(files
            .into_iter()
            .filter(|file| file.filename != "." && file.filename != "..")
            .map(|file| Entry {
                path: path.join(&file.filename),
                meta: meta(&file.attrs),
            })
            .collect())
    }

    fn listing_is_complete(&self) -> bool {
        // OpenSSH fills each listing entry from `lstat`, and `stat` above is
        // `lstat`, so the two say the same. The integration suite checks it.
        true
    }

    fn open_read(&self, path: &Path) -> Result<Box<dyn Read + Send>> {
        Ok(Box::new(self.reader(path)?))
    }

    fn read_prefix(&self, path: &Path, len: u64) -> Result<Vec<u8>> {
        self.read_range(path, 0, len)
    }

    fn read_range(&self, path: &Path, offset: u64, len: u64) -> Result<Vec<u8>> {
        let mut reader = self.reader(path)?;
        // Every SFTP read names its offset, so starting further in is free.
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
        // Version 3 carries 32-bit seconds; a time it cannot say is one it
        // cannot set, which is an answer rather than a failure.
        let Some(seconds) = at
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|since| u32::try_from(since.as_secs()).ok())
        else {
            return Ok(false);
        };
        let name = self.name(path)?;
        // Access and modification times travel together in version 3.
        let attrs = FileAttributes {
            atime: Some(seconds),
            mtime: Some(seconds),
            ..FileAttributes::empty()
        };
        self.call("set_modified", path, move |live| async move {
            live.sftp.setstat(name, attrs).await.map(|_| true)
        })
    }

    fn create_write(&self, path: &Path) -> Result<Box<dyn WriteFinish>> {
        let name = self.name(path)?;
        let live = self.live()?;
        let session = Arc::clone(&live);
        let handle = dispatch(async move {
            session
                .sftp
                .open(
                    name,
                    OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE,
                    FileAttributes::default(),
                )
                .await
                .map(|opened| opened.handle)
        })
        .map_err(|error| self.failure("write", path, &error))?;
        Ok(Box::new(SftpWrite {
            live,
            handle: Some(handle),
            offset: 0,
            path: path.to_path_buf(),
            endpoint: self.endpoint.clone(),
        }))
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let (source, target) = (self.name(from)?, self.name(to)?);
        let replacing = self.posix_rename;
        self.call("rename", from, move |live| async move {
            if !replacing {
                return live.sftp.rename(source, target).await.map(|_| ());
            }
            // Same two strings on the wire as OpenSSH's hardlink request,
            // which is why that struct serves here.
            let data: Vec<u8> = HardlinkExtension {
                oldpath: source,
                newpath: target,
            }
            .try_into()?;
            match live.sftp.extended("posix-rename@openssh.com", data).await? {
                Packet::Status(status) if status.status_code == StatusCode::Ok => Ok(()),
                Packet::Status(status) => Err(SftpError::Status(status)),
                _ => Err(SftpError::UnexpectedPacket),
            }
        })
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        let name = self.name(path)?;
        self.call("delete", path, move |live| async move {
            live.sftp.remove(name).await.map(|_| ())
        })
    }

    fn remove_dir(&self, path: &Path) -> Result<()> {
        let name = self.name(path)?;
        // The server refuses a folder that is not empty, which is the contract.
        self.call("delete", path, move |live| async move {
            live.sftp.rmdir(name).await.map(|_| ())
        })
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        // The same rail as every other backend: never build a folder chain
        // on a server whose folder has gone.
        self.root_token()?;
        // The anchor exists, as `root_token` just proved. Everything after it
        // is made in turn, each found or created.
        let mut so_far = self.anchor.clone();
        for part in self.inside.iter().cloned().chain(path::parts(path)?) {
            so_far = path::under(&so_far, &[part]);
            self.make_folder(so_far.clone(), path)?;
        }
        Ok(())
    }
}

/// A file being read, many requests in flight at a time.
struct SftpRead {
    live: Arc<Live>,
    /// `None` once closed.
    handle: Option<String>,
    /// Where the next request reads from.
    offset: u64,
    /// What has arrived and not been handed on yet.
    ready: Vec<u8>,
    taken: usize,
    done: bool,
}

impl SftpRead {
    /// Ask for the next stretch of the file, [`PIECES_AT_ONCE`] pieces at once.
    fn fill(&mut self) -> std::io::Result<()> {
        let Some(handle) = self.handle.clone() else {
            self.done = true;
            return Ok(());
        };
        let live = Arc::clone(&self.live);
        let start = self.offset;
        let answers = dispatch(async move {
            let asks = (0..PIECES_AT_ONCE).map(|n| {
                let offset = start + (n * PIECE) as u64;
                live.sftp.read(handle.clone(), offset, PIECE_LEN)
            });
            join_all(asks).await
        });
        let mut got = Vec::with_capacity(PIECES_AT_ONCE * PIECE);
        for answer in answers {
            match answer {
                Ok(data) => {
                    let short = data.data.len() < PIECE;
                    got.extend_from_slice(&data.data);
                    // A short answer may be the end or only a server's limit.
                    // Either way the pieces asked for after it start in the
                    // wrong place, so they are dropped and asked for again.
                    if short {
                        self.done = data.data.is_empty();
                        break;
                    }
                }
                Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                    self.done = true;
                    break;
                }
                Err(error) => return Err(std::io::Error::other(error.to_string())),
            }
        }
        if got.is_empty() {
            self.done = true;
        }
        self.offset += got.len() as u64;
        self.ready = got;
        self.taken = 0;
        Ok(())
    }

    fn close(&mut self) {
        if let Some(handle) = self.handle.take() {
            let live = Arc::clone(&self.live);
            let _ = dispatch(async move { live.sftp.close(handle).await });
        }
    }
}

impl Read for SftpRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.taken == self.ready.len() {
            if self.done {
                self.close();
                return Ok(0);
            }
            self.fill()?;
        }
        let take = buf.len().min(self.ready.len() - self.taken);
        buf[..take].copy_from_slice(&self.ready[self.taken..self.taken + take]);
        self.taken += take;
        Ok(take)
    }
}

impl Drop for SftpRead {
    fn drop(&mut self) {
        self.close();
    }
}

/// A file being written, committed by `finish`.
struct SftpWrite {
    live: Arc<Live>,
    handle: Option<String>,
    offset: u64,
    path: PathBuf,
    endpoint: String,
}

impl Write for SftpWrite {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let Some(handle) = self.handle.clone() else {
            return Err(std::io::Error::other("this writer has already been closed"));
        };
        let live = Arc::clone(&self.live);
        let start = self.offset;
        let pieces: Vec<(u64, Vec<u8>)> = buf
            .chunks(PIECE)
            .enumerate()
            .map(|(n, piece)| (start + (n * PIECE) as u64, piece.to_vec()))
            .collect();
        // Every piece of the buffer at once; SFTP answers each by number.
        let answers = dispatch(async move {
            join_all(
                pieces
                    .into_iter()
                    .map(|(offset, piece)| live.sftp.write(handle.clone(), offset, piece)),
            )
            .await
        });
        for answer in answers {
            answer.map_err(|error| std::io::Error::other(error.to_string()))?;
        }
        self.offset += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // What makes it durable is `finish`, as for a local file.
        Ok(())
    }
}

impl WriteFinish for SftpWrite {
    fn finish(mut self: Box<Self>) -> Result<()> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        let live = Arc::clone(&self.live);
        // Put the bytes on the server's disk where it can be asked to, then
        // close, which is when the file is complete for anyone else. Both
        // must succeed before the journal may say this file arrived.
        let finished = dispatch(async move {
            let synced = if live.fsync {
                live.sftp.fsync(handle.clone()).await.map(|_| ())
            } else {
                Ok(())
            };
            let closed = live.sftp.close(handle).await.map(|_| ());
            synced.and(closed)
        });
        finished.map_err(|error| BackendError::Remote {
            endpoint: self.endpoint.clone(),
            operation: "close",
            source: Box::new(std::io::Error::other(format!(
                "`{}` did not finish: {error}",
                self.path.display()
            ))),
        })
    }
}

impl Drop for SftpWrite {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let live = Arc::clone(&self.live);
            let _ = dispatch(async move { live.sftp.close(handle).await });
        }
    }
}

#[cfg(test)]
mod tests;
