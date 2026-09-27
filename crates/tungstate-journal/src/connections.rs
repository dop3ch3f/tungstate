//! Connections: the named places a link end can live, other than this machine.
//!
//! A link end used to be a bare path, which quietly assumed the local
//! filesystem. A connection is the other half of that: the *place*, with the
//! path becoming relative to it. `None` is still the local filesystem, so every
//! link written before this existed means exactly what it always meant.
//!
//! **No secret is stored here.** A connection records where and as whom, never
//! the password; that lives in the machine's keychain, keyed by name. The
//! absence is structural — there is no column to put one in.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::{Journal, JournalError, Result, now_millis, query};

/// Identifier for a configured connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnectionId(pub i64);

/// Which protocol a connection speaks.
///
/// Deliberately a closed enum rather than a free-text scheme: the factory has
/// to know how to build an operator for every value, so an unknown one is a
/// bug, not a configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// A directory on some filesystem this machine can already reach, driven
    /// through `OpenDAL` rather than through `std::fs`.
    Fs,
    /// File Transfer Protocol, in the clear.
    Ftp,
    /// FTP over TLS. Its own value rather than an option on [`Scheme::Ftp`]
    /// because plain FTP sends the password across the wire in clear text, so
    /// which one a connection uses has to be visible at a glance in
    /// `connection list`, not buried in an options blob.
    Ftps,
    /// Anything that speaks the S3 API: AWS, Backblaze B2, Cloudflare R2,
    /// `MinIO`, a NAS's own object store. The bucket, region and endpoint are
    /// options; the access key id is the username and the secret key is the
    /// password.
    S3,
    /// Windows file sharing, reached directly rather than through a mount.
    /// The server is the host, and the root is the share followed by the
    /// folder inside it.
    Smb,
}

// The same spellings serve the database and the CLI, so a stored value is
// always a value the user could have typed. New values are new strings, so a
// journal written before them reads exactly as it did.
string_enum!(Scheme { Fs => "fs", Ftp => "ftp", Ftps => "ftps", S3 => "s3", Smb => "smb" });

/// Keys in a connection's options that some scheme reads.
pub mod option {
    /// S3: the bucket every path lives in. Required.
    pub const BUCKET: &str = "bucket";
    /// S3: the bucket's region. Found by asking AWS when not given.
    pub const REGION: &str = "region";
    /// S3: where the service is, for anything that is not AWS itself.
    pub const ENDPOINT: &str = "endpoint";
    /// SMB: `required` to refuse a server that will not encrypt. Left to the
    /// server otherwise, which is what Finder and Windows do.
    pub const ENCRYPTION: &str = "encryption";
}

impl Scheme {
    /// Whether this scheme has a password worth keeping in the keychain.
    #[must_use]
    pub fn authenticates(self) -> bool {
        match self {
            Self::Fs => false,
            Self::Ftp | Self::Ftps | Self::S3 | Self::Smb => true,
        }
    }

    /// Whether credentials and content are encrypted in transit, by the
    /// scheme alone. A connection can say otherwise either way; ask
    /// [`ConnectionSettings::in_the_clear`] about a real one.
    #[must_use]
    pub fn is_encrypted(self) -> bool {
        match self {
            // Local, so nothing crosses a wire in the first place.
            Self::Fs | Self::Ftps | Self::S3 => true,
            // SMB leaves encrypting the contents to the server unless told
            // to insist, so it is not encrypted until a connection says so.
            Self::Ftp | Self::Smb => false,
        }
    }

    /// Whether this scheme can move a file into place without copying it.
    ///
    /// False for FTP not because the protocol lacks `RNFR`/`RNTO` but because
    /// `OpenDAL`'s FTP service answers `rename` with `Unsupported`. The CLI uses
    /// this to refuse `--on-conflict replace` when the link is created rather
    /// than at the first clash; the engine checks the backend's own
    /// capabilities, which is the authority.
    #[must_use]
    pub fn can_rename(self) -> bool {
        match self {
            Self::Fs | Self::Smb => true,
            // An object store has no rename at all: a new name is a copy.
            Self::Ftp | Self::Ftps | Self::S3 => false,
        }
    }

    /// Whether the far side can checksum a file for us.
    ///
    /// FTP cannot, so `--verify hash` there only ever attests to the bytes
    /// that were sent, not to the bytes that landed.
    #[must_use]
    pub fn has_native_checksum(self) -> bool {
        match self {
            // Not a checksum, but reading the file back is free and local, so
            // `hash` is not misleading the way it is over a network.
            Self::Fs => true,
            // An S3 ETag is an MD5 only for single-part uploads, so it cannot
            // be trusted as a checksum of what landed.
            Self::Ftp | Self::Ftps | Self::S3 | Self::Smb => false,
        }
    }

    /// What an empty root means, for a connection that has one.
    ///
    /// `OpenDAL` normalises an empty root to `/`, so a networked connection
    /// with no root writes relative to the *server's* root directory. On a NAS
    /// that is almost never where the account can write, and the failure
    /// arrives much later as a refused write on the first file, which is the
    /// wrong moment to learn it. `None` when there is nothing to warn about.
    #[must_use]
    pub fn rootless_warning(self, root: &str) -> Option<&'static str> {
        // An S3 root is a prefix inside a bucket and SMB's starts with the
        // share, which `ConnectionSettings::problems` insists on; neither has
        // a server-wide `/` to land in.
        let has_server_root = matches!(self, Self::Ftp | Self::Ftps);
        (has_server_root && root.trim().is_empty()).then_some(concat!(
            "no root was given, so paths are taken from the server's own `/`.\n",
            "      That is rarely where the account can write, and you would find out at\n",
            "      the first file. If your login lands in a folder, set that as the root.\n",
            "      `connection test` prints the directory it is actually using."
        ))
    }

    /// Whether reaching this place costs a network connection.
    ///
    /// What decides how many transfers may run at once: a filesystem has no
    /// per-client connection limit to exceed, and a protocol does.
    #[must_use]
    pub fn is_networked(self) -> bool {
        match self {
            Self::Fs => false,
            Self::Ftp | Self::Ftps | Self::S3 | Self::Smb => true,
        }
    }

    /// The port used when the connection does not name one.
    #[must_use]
    pub fn default_port(self) -> Option<u16> {
        match self {
            // A folder has no port, and an S3 endpoint is a URL that carries
            // its own.
            Self::Fs | Self::S3 => None,
            // Explicit FTPS (`AUTH TLS`) upgrades an ordinary control
            // connection, so it uses 21 too. Implicit FTPS on 990 is legacy
            // and servers that need it can be given `--port 990`.
            Self::Ftp | Self::Ftps => Some(21),
            Self::Smb => Some(445),
        }
    }

    /// Every scheme, in the order a person choosing one should see them.
    pub const ALL: [Self; 5] = [Self::Fs, Self::Smb, Self::Ftps, Self::Ftp, Self::S3];
}

/// A scheme spelling this build does not know, which means a newer tungstate
/// wrote the row.
#[derive(Debug, thiserror::Error)]
#[error("unknown connection scheme `{0}`")]
struct UnknownScheme(String);

/// A configured connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    /// Assigned when the connection is created.
    pub id: ConnectionId,
    /// The name used on the command line, and the keychain account suffix.
    pub name: String,
    /// Which protocol this speaks.
    pub scheme: Scheme,
    /// Hostname, for the schemes that have one.
    pub host: Option<String>,
    /// Port, where it differs from the protocol default.
    pub port: Option<u16>,
    /// Who we connect as, for the schemes that authenticate.
    pub username: Option<String>,
    /// The directory on the far side that every path is relative to.
    pub root: String,
    /// Per-scheme extras, passed through to the backend factory.
    pub options: BTreeMap<String, String>,
    /// When it was created, in milliseconds since the Unix epoch.
    pub created_at: i64,
}

/// The fields needed to create a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewConnection {
    /// The name it will be referred to by. Must be unique.
    pub name: String,
    /// Which protocol this speaks.
    pub scheme: Scheme,
    /// Hostname, for the schemes that have one.
    pub host: Option<String>,
    /// Port, where it differs from the protocol default.
    pub port: Option<u16>,
    /// Who we connect as, for the schemes that authenticate.
    pub username: Option<String>,
    /// The directory on the far side that every path is relative to.
    pub root: String,
    /// Per-scheme extras, passed through to the backend factory.
    pub options: BTreeMap<String, String>,
}

/// Everything about a connection that can change after it is created.
///
/// The name is the key, not a field: it is what people typed into their link
/// specs, and it derives the keychain account, so renaming is two migrations
/// wearing one hat. Encoding that in the type is the point — this API cannot
/// desynchronise the keychain from the journal because it cannot express a
/// rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSettings {
    /// Which protocol this speaks.
    pub scheme: Scheme,
    /// Hostname, for the schemes that have one.
    pub host: Option<String>,
    /// Port, where it differs from the protocol default.
    pub port: Option<u16>,
    /// Who we connect as, for the schemes that authenticate.
    pub username: Option<String>,
    /// The directory on the far side that every path is relative to.
    pub root: String,
    /// Per-scheme extras, passed through to the backend factory.
    pub options: BTreeMap<String, String>,
}

impl From<&Connection> for ConnectionSettings {
    fn from(connection: &Connection) -> Self {
        Self {
            scheme: connection.scheme,
            host: connection.host.clone(),
            port: connection.port,
            username: connection.username.clone(),
            root: connection.root.clone(),
            options: connection.options.clone(),
        }
    }
}

/// Something in a connection's settings that would stop it working, found
/// before it is saved rather than at the first file.
///
/// Data rather than a sentence, so the command line and the window refuse the
/// same things and each can say it in its own place.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsProblem {
    /// A folder on this machine with no path.
    #[error("a folder on this machine needs its path as the root")]
    NeedsRoot,
    /// A networked scheme with nowhere to connect to.
    #[error("this needs the server's name or address as the host")]
    NeedsHost,
    /// S3 with no bucket.
    #[error("S3 needs a bucket")]
    NeedsBucket,
    /// SMB with no share at the start of the root.
    #[error("the root has to start with the share's name, as in media/backups")]
    NeedsShare,
    /// An S3 endpoint that is not a web address.
    #[error("the endpoint has to start with https:// or http://, and `{0}` does not")]
    BadEndpoint(String),
    /// A value outside the ones an option accepts.
    #[error("`{value}` is not a choice for {key}; use {expected}")]
    BadValue {
        /// The option.
        key: &'static str,
        /// What was given.
        value: String,
        /// What would have been accepted, as words.
        expected: &'static str,
    },
    /// An option some other scheme reads, which this one would ignore.
    #[error("{0} means nothing to this kind of connection")]
    UnusedOption(&'static str),
}

impl ConnectionSettings {
    /// Everything that would stop these settings working. Empty means they
    /// are worth saving and testing.
    #[must_use]
    pub fn problems(&self) -> Vec<SettingsProblem> {
        let mut found = Vec::new();
        let given = |key: &str| {
            self.options
                .get(key)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
        };
        let host = self
            .host
            .as_deref()
            .is_some_and(|host| !host.trim().is_empty());

        match self.scheme {
            Scheme::Fs if self.root.trim().is_empty() => found.push(SettingsProblem::NeedsRoot),
            Scheme::Ftp | Scheme::Ftps | Scheme::Smb if !host => {
                found.push(SettingsProblem::NeedsHost);
            }
            _ => {}
        }
        if self.scheme == Scheme::Smb && share_of(&self.root).is_none() {
            found.push(SettingsProblem::NeedsShare);
        }
        if self.scheme == Scheme::S3 {
            if given(option::BUCKET).is_none() {
                found.push(SettingsProblem::NeedsBucket);
            }
            if let Some(endpoint) = given(option::ENDPOINT) {
                let lower = endpoint.to_ascii_lowercase();
                if !(lower.starts_with("https://") || lower.starts_with("http://")) {
                    found.push(SettingsProblem::BadEndpoint(endpoint.to_string()));
                }
            }
        }
        if let Some(value) = given(option::ENCRYPTION)
            && self.scheme == Scheme::Smb
            && value != "required"
        {
            found.push(SettingsProblem::BadValue {
                key: option::ENCRYPTION,
                value: value.to_string(),
                expected: "required, or leave it out",
            });
        }

        // Only the keys some scheme reads. An unknown key may be a newer
        // build's, and refusing it would stop an older one saving anything.
        let reads = |key: &str| match key {
            option::BUCKET | option::REGION | option::ENDPOINT => self.scheme == Scheme::S3,
            option::ENCRYPTION => self.scheme == Scheme::Smb,
            _ => true,
        };
        for key in [
            option::BUCKET,
            option::REGION,
            option::ENDPOINT,
            option::ENCRYPTION,
        ] {
            if given(key).is_some() && !reads(key) {
                found.push(SettingsProblem::UnusedOption(key));
            }
        }
        found
    }

    /// What crosses the network unencrypted with these settings, as advice,
    /// or `None` when nothing does.
    ///
    /// The scheme alone cannot say: S3 is encrypted unless its endpoint is
    /// plain `http://`, and SMB only when the server or the connection
    /// insists.
    #[must_use]
    pub fn in_the_clear(&self) -> Option<&'static str> {
        let option = |key: &str| {
            self.options
                .get(key)
                .map(|value| value.trim().to_ascii_lowercase())
        };
        match self.scheme {
            Scheme::Fs | Scheme::Ftps => None,
            Scheme::Ftp => Some(
                "sends your password and your files across the network unencrypted. \
                 If the server offers it, use ftps instead.",
            ),
            Scheme::S3 => option(option::ENDPOINT)
                .is_some_and(|endpoint| endpoint.starts_with("http://"))
                .then_some(
                    "the endpoint is plain http, so your files cross the network \
                     unencrypted. Your secret key never does. Use an https:// endpoint \
                     if the service has one.",
                ),
            Scheme::Smb => (option(option::ENCRYPTION).as_deref() != Some("required")).then_some(
                "files cross the network unencrypted unless the server turns on SMB \
                 encryption. Your password is never sent as it is. Set encryption to \
                 required to refuse a server that will not encrypt.",
            ),
        }
    }
}

/// The share an SMB root starts with, if it names one.
///
/// Either slash separates, because a Windows user will type `media\backups`.
#[must_use]
pub fn share_of(root: &str) -> Option<&str> {
    root.split(['/', '\\']).find(|part| !part.trim().is_empty())
}

/// One end of a link: which place, and where inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// `None` is the local filesystem.
    pub connection: Option<ConnectionId>,
    /// Absolute for local; relative to the connection's root for a remote.
    pub path: PathBuf,
}

impl Endpoint {
    /// An end on this machine's own filesystem.
    #[must_use]
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self {
            connection: None,
            path: path.into(),
        }
    }

    /// An end inside a named connection.
    #[must_use]
    pub fn remote(connection: ConnectionId, path: impl Into<PathBuf>) -> Self {
        Self {
            connection: Some(connection),
            path: path.into(),
        }
    }

    /// True when this end is somewhere other than the local filesystem.
    #[must_use]
    pub fn is_remote(&self) -> bool {
        self.connection.is_some()
    }
}

impl Journal {
    /// Create a connection.
    ///
    /// # Errors
    /// [`JournalError::DuplicateConnection`] if the name is taken, or
    /// [`JournalError::Query`] if the row cannot be written.
    pub fn create_connection(&self, connection: &NewConnection) -> Result<ConnectionId> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO connections (
                 name, scheme, host, port, username, root, options, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                connection.name,
                connection.scheme.as_str(),
                connection.host,
                connection.port.map(i64::from),
                connection.username,
                connection.root,
                encode_options(&connection.options),
                now_millis(),
            ],
        )
        .map_err(|error| match error {
            rusqlite::Error::SqliteFailure(inner, _)
                if inner.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                JournalError::DuplicateConnection(connection.name.clone())
            }
            other => JournalError::Query {
                context: "creating a connection",
                source: other,
            },
        })?;
        Ok(ConnectionId(conn.last_insert_rowid()))
    }

    /// Look a connection up by the name the user typed.
    ///
    /// # Errors
    /// [`JournalError::UnknownConnection`] if there is no such connection, or
    /// [`JournalError::Query`] if the row cannot be read.
    pub fn connection_by_name(&self, name: &str) -> Result<Connection> {
        let conn = self.lock();
        conn.query_row(
            "SELECT * FROM connections WHERE name = ?1",
            rusqlite::params![name],
            row_to_connection,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                JournalError::UnknownConnection(name.to_string())
            }
            other => JournalError::Query {
                context: "looking up a connection",
                source: other,
            },
        })
    }

    /// Look a connection up by the id stored on a link end.
    ///
    /// # Errors
    /// [`JournalError::UnknownConnection`] if the row is gone, or
    /// [`JournalError::Query`] if it cannot be read.
    pub fn connection_by_id(&self, id: ConnectionId) -> Result<Connection> {
        let conn = self.lock();
        conn.query_row(
            "SELECT * FROM connections WHERE id = ?1",
            rusqlite::params![id.0],
            row_to_connection,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                JournalError::UnknownConnection(format!("#{}", id.0))
            }
            other => JournalError::Query {
                context: "looking up a connection by id",
                source: other,
            },
        })
    }

    /// Replace everything about a connection that can change.
    ///
    /// A full replace of the mutable columns rather than a partial patch: a
    /// form submits every field anyway, and partial-update semantics over
    /// `Option<Option<T>>` is a well-known way to make "clear this field"
    /// unreachable. The CLI layers its `--flag`-shaped partial UX on top.
    ///
    /// Allowed while links point at it, where deleting is refused: a link
    /// references a connection by id, so editing where the connection goes
    /// re-points every link at once, which is the whole reason to have the
    /// indirection.
    ///
    /// # Errors
    /// [`JournalError::UnknownConnection`] if there is no such row, or
    /// [`JournalError::Query`] if the update fails.
    pub fn update_connection(&self, name: &str, settings: &ConnectionSettings) -> Result<()> {
        let conn = self.lock();
        let changed = conn
            .execute(
                "UPDATE connections
                    SET scheme = ?2, host = ?3, port = ?4,
                        username = ?5, root = ?6, options = ?7
                  WHERE name = ?1",
                rusqlite::params![
                    name,
                    settings.scheme.as_str(),
                    settings.host,
                    settings.port.map(i64::from),
                    settings.username,
                    settings.root,
                    encode_options(&settings.options),
                ],
            )
            .map_err(query("updating a connection"))?;
        if changed == 0 {
            return Err(JournalError::UnknownConnection(name.to_string()));
        }
        Ok(())
    }

    /// Names of the links pointing at a connection, so a refused delete can
    /// say which.
    ///
    /// [`Journal::delete_connection`] leans on the foreign key, which knows
    /// that something references the row but not what. "Remove these two
    /// links first" is actionable where "it is in use" is a guessing game.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn links_using(&self, id: ConnectionId) -> Result<Vec<String>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT name FROM links
                  WHERE source_connection = ?1 OR dest_connection = ?1
                  ORDER BY name",
            )
            .map_err(query("listing the links using a connection"))?;
        let names = stmt
            .query_map(rusqlite::params![id.0], |row| row.get(0))
            .map_err(query("listing the links using a connection"))?
            .collect::<std::result::Result<Vec<String>, _>>()
            .map_err(query("listing the links using a connection"))?;
        Ok(names)
    }

    /// Every configured connection, in creation order.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn connections(&self) -> Result<Vec<Connection>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT * FROM connections ORDER BY id")
            .map_err(query("listing connections"))?;
        let rows = stmt
            .query_map([], row_to_connection)
            .map_err(query("listing connections"))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(query("listing connections"))?;
        Ok(rows)
    }

    /// Delete a connection.
    ///
    /// Refused while any link still points at it. That refusal comes from the
    /// foreign key rather than from a check here, so it holds even against a
    /// caller who forgets to look.
    ///
    /// # Errors
    /// [`JournalError::ConnectionInUse`] if a link still references it,
    /// [`JournalError::UnknownConnection`] if there is no such row, or
    /// [`JournalError::Query`] if the delete fails.
    pub fn delete_connection(&self, name: &str) -> Result<()> {
        let conn = self.lock();
        let removed = conn
            .execute(
                "DELETE FROM connections WHERE name = ?1",
                rusqlite::params![name],
            )
            .map_err(|error| match error {
                rusqlite::Error::SqliteFailure(inner, _)
                    if inner.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    JournalError::ConnectionInUse(name.to_string())
                }
                other => JournalError::Query {
                    context: "deleting a connection",
                    source: other,
                },
            })?;
        if removed == 0 {
            return Err(JournalError::UnknownConnection(name.to_string()));
        }
        Ok(())
    }
}

/// Options travel as a JSON object so the column stays one value however many
/// per-scheme knobs a later backend needs.
fn encode_options(options: &BTreeMap<String, String>) -> String {
    serde_json::to_string(options).unwrap_or_else(|_| "{}".to_string())
}

/// Unreadable options mean a newer tungstate wrote something this build does not
/// understand. Empty is the safe reading: the connection still opens with
/// defaults rather than refusing to load at all.
fn decode_options(raw: &str) -> BTreeMap<String, String> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn row_to_connection(row: &rusqlite::Row<'_>) -> rusqlite::Result<Connection> {
    let scheme_raw: String = row.get("scheme")?;
    let options_raw: String = row.get("options")?;
    // Unlike the link enums, an unknown scheme has no safe fallback: reading a
    // future protocol as "fs" would point a drain at the local disk. Refuse the
    // row instead.
    let scheme = Scheme::parse(&scheme_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(UnknownScheme(scheme_raw)),
        )
    })?;
    Ok(Connection {
        id: ConnectionId(row.get("id")?),
        name: row.get("name")?,
        scheme,
        host: row.get("host")?,
        port: row
            .get::<_, Option<i64>>("port")?
            .and_then(|p| u16::try_from(p).ok()),
        username: row.get("username")?,
        root: row.get("root")?,
        options: decode_options(&options_raw),
        created_at: row.get("created_at")?,
    })
}
