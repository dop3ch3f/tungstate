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
}

// The same spellings serve the database and the CLI, so a stored value is
// always a value the user could have typed.
string_enum!(Scheme { Fs => "fs", Ftp => "ftp", Ftps => "ftps" });

impl Scheme {
    /// Whether this scheme has a password worth keeping in the keychain.
    #[must_use]
    pub fn authenticates(self) -> bool {
        match self {
            Self::Fs => false,
            Self::Ftp | Self::Ftps => true,
        }
    }

    /// Whether credentials and content are encrypted in transit.
    #[must_use]
    pub fn is_encrypted(self) -> bool {
        match self {
            // Local, so nothing crosses a wire in the first place.
            Self::Fs | Self::Ftps => true,
            Self::Ftp => false,
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
            Self::Fs => true,
            Self::Ftp | Self::Ftps => false,
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
            Self::Ftp | Self::Ftps => false,
        }
    }

    /// Whether reaching this place costs a network connection.
    ///
    /// What decides how many transfers may run at once: a filesystem has no
    /// per-client connection limit to exceed, and a protocol does.
    #[must_use]
    pub fn is_networked(self) -> bool {
        match self {
            Self::Fs => false,
            Self::Ftp | Self::Ftps => true,
        }
    }

    /// The port used when the connection does not name one.
    #[must_use]
    pub fn default_port(self) -> Option<u16> {
        match self {
            Self::Fs => None,
            // Explicit FTPS (`AUTH TLS`) upgrades an ordinary control
            // connection, so it uses 21 too. Implicit FTPS on 990 is legacy
            // and servers that need it can be given `--port 990`.
            Self::Ftp | Self::Ftps => Some(21),
        }
    }
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
