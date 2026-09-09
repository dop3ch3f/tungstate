//! Append-only record of every operation tungstate performs.
//!
//! Two jobs. It answers "where did this file go?" for the user, and it is what
//! makes a half-finished drain recoverable: every operation is written down as
//! [`OpStatus::Intended`] *before* it touches the filesystem, so anything still
//! marked intended at startup is work that was interrupted.
//!
//! One journal per machine. A drain moves a file from one folder to another,
//! which is a single story, and splitting it across per-folder databases would
//! make [`Journal::whereis`] open every database on the machine to answer one
//! question.

mod schema;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

/// Anything that can go wrong reading or writing the journal.
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    /// The journal database could not be opened or created.
    #[error("could not open journal at `{path}`")]
    Open {
        /// Where the journal was expected to live.
        path: PathBuf,
        /// The underlying SQLite failure.
        #[source]
        source: rusqlite::Error,
    },

    /// A query or statement failed.
    #[error("journal query failed: {context}")]
    Query {
        /// What was being attempted, for a log a human has to read.
        context: &'static str,
        /// The underlying SQLite failure.
        #[source]
        source: rusqlite::Error,
    },

    /// The journal's parent directory could not be created.
    #[error("could not create the journal directory `{path}`")]
    CreateDir {
        /// The directory that could not be created.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },

    /// The platform has no standard state directory.
    #[error("could not determine a state directory for this platform")]
    NoStateDir,

    /// A caller referred to an operation that is not in the journal.
    #[error("no operation with id {0}")]
    UnknownOp(i64),
}

/// Result alias so signatures read `Result<Op>` rather than spelling out the error.
pub type Result<T> = std::result::Result<T, JournalError>;

/// Identifier for one recorded operation.
///
/// A newtype so an operation id cannot be passed where a folder id belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OpId(pub i64);

/// What kind of work an operation represents.
///
/// Stored as text rather than an integer so `sqlite3 journal.db` is readable by
/// a human debugging a stuck drain at midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    /// Content duplicated, source left in place.
    Copy,
    /// Content transferred, source removed once verified.
    Move,
    /// Renamed or relocated within one backend.
    Rename,
    /// Deleted, or moved to trash.
    Remove,
    /// Directory created.
    MkDir,
}

/// How an operation ended, or that it has not ended yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpStatus {
    /// Written down, not yet performed. Anything still here at startup was interrupted.
    Intended,
    /// Completed and verified.
    Committed,
    /// Attempted and failed. `error` explains why.
    Failed,
    /// Deliberately not performed, for example an exact duplicate already present.
    Skipped,
}

impl OpKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
            Self::Rename => "rename",
            Self::Remove => "remove",
            Self::MkDir => "mkdir",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "copy" => Some(Self::Copy),
            "move" => Some(Self::Move),
            "rename" => Some(Self::Rename),
            "remove" => Some(Self::Remove),
            "mkdir" => Some(Self::MkDir),
            _ => None,
        }
    }
}

impl OpStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Intended => "intended",
            Self::Committed => "committed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "intended" => Some(Self::Intended),
            "committed" => Some(Self::Committed),
            "failed" => Some(Self::Failed),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

/// One end of an operation: which storage location, and where inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// Backend root, so two folders with a `videos/` subdirectory stay distinct.
    pub root: PathBuf,
    /// Path relative to that root.
    pub path: PathBuf,
}

impl Location {
    /// A location within `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, path: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            path: path.into(),
        }
    }

    /// The full path this location denotes.
    #[must_use]
    pub fn full(&self) -> PathBuf {
        self.root.join(&self.path)
    }
}

/// An operation about to be attempted.
#[derive(Debug, Clone)]
pub struct NewOp {
    /// What kind of work this is.
    pub kind: OpKind,
    /// Where the content comes from. `None` for operations that create something.
    pub source: Option<Location>,
    /// Where the content is going. `None` for removals.
    pub destination: Option<Location>,
    /// Size in bytes where known.
    pub size: Option<u64>,
    /// Which transfer link asked for this, feeding the `{Source}` policy variable.
    pub link: Option<String>,
}

/// How an operation turned out.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// It worked. Carries the content hash if one was computed.
    Committed {
        /// BLAKE3 hex digest, once slice 3 computes one.
        hash: Option<String>,
    },
    /// It failed, with a human-readable reason.
    Failed {
        /// Why it failed.
        error: String,
    },
    /// It was deliberately not done.
    Skipped {
        /// Why it was skipped.
        reason: String,
    },
}

/// One operation as recorded.
#[derive(Debug, Clone)]
pub struct Op {
    /// Identifier assigned when the intent was written.
    pub id: OpId,
    /// What kind of work this is.
    pub kind: OpKind,
    /// Where it has got to.
    pub status: OpStatus,
    /// Where the content came from.
    pub source: Option<Location>,
    /// Where the content went.
    pub destination: Option<Location>,
    /// Size in bytes where known.
    pub size: Option<u64>,
    /// BLAKE3 hex digest where computed.
    pub hash: Option<String>,
    /// Which transfer link asked for this.
    pub link: Option<String>,
    /// Failure reason or skip reason.
    pub note: Option<String>,
    /// When the intent was written, in milliseconds since the Unix epoch.
    pub started_at: i64,
    /// When the outcome was recorded, if it has been.
    pub finished_at: Option<i64>,
}

/// What to look for when asking where something is.
#[derive(Debug, Clone)]
pub enum Locator<'a> {
    /// A path, matched against both sources and destinations.
    Path(&'a Path),
    /// A BLAKE3 hex digest.
    Hash(&'a str),
}

/// The journal database.
///
/// Cheap to clone references to; share one instance rather than opening several,
/// since SQLite permits a single writer.
#[derive(Debug)]
pub struct Journal {
    // rusqlite::Connection is Send but not Sync, and slice 4 writes from several
    // threads. SQLite serialises writers anyway, so the mutex costs nothing real.
    conn: Mutex<Connection>,
}

impl Journal {
    /// Open, creating it and its parent directory if needed.
    ///
    /// # Errors
    /// [`JournalError::CreateDir`] if the parent directory cannot be made, or
    /// [`JournalError::Open`] if the file cannot be opened or the schema cannot
    /// be brought up to date.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| JournalError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let conn = Connection::open(path).map_err(|source| JournalError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        schema::prepare(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open at this machine's standard state directory.
    ///
    /// # Errors
    /// [`JournalError::NoStateDir`] if the platform has no such directory, or
    /// [`JournalError::Open`] as [`Journal::open`].
    pub fn open_default() -> Result<Self> {
        Self::open(&default_path()?)
    }

    /// Open a throwaway journal held entirely in memory, for tests.
    ///
    /// # Errors
    /// [`JournalError::Open`] if SQLite cannot create the database.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(|source| JournalError::Open {
            path: PathBuf::from(":memory:"),
            source,
        })?;
        schema::prepare(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Record what is about to be attempted, before attempting it.
    ///
    /// This ordering is the whole point of the journal. A row left as
    /// [`OpStatus::Intended`] is how a resumed drain recognises interrupted work.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the row cannot be written.
    pub fn begin(&self, op: &NewOp) -> Result<OpId> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO ops (
                 kind, status, src_root, src_path, dst_root, dst_path,
                 size, link, started_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                op.kind.as_str(),
                OpStatus::Intended.as_str(),
                op.source.as_ref().map(|l| path_str(&l.root)),
                op.source.as_ref().map(|l| path_str(&l.path)),
                op.destination.as_ref().map(|l| path_str(&l.root)),
                op.destination.as_ref().map(|l| path_str(&l.path)),
                op.size.map(size_to_sql),
                op.link,
                now_millis(),
            ],
        )
        .map_err(query("recording an intended operation"))?;
        Ok(OpId(conn.last_insert_rowid()))
    }

    /// Record how an operation turned out.
    ///
    /// # Errors
    /// [`JournalError::UnknownOp`] if `id` was never begun, or
    /// [`JournalError::Query`] if the update fails.
    pub fn finish(&self, id: OpId, outcome: &Outcome) -> Result<()> {
        let (status, hash, note) = match outcome {
            Outcome::Committed { hash } => (OpStatus::Committed, hash.clone(), None),
            Outcome::Failed { error } => (OpStatus::Failed, None, Some(error.clone())),
            Outcome::Skipped { reason } => (OpStatus::Skipped, None, Some(reason.clone())),
        };

        let conn = self.lock();
        let changed = conn
            .execute(
                "UPDATE ops SET status = ?1, hash = ?2, note = ?3, finished_at = ?4 WHERE id = ?5",
                rusqlite::params![status.as_str(), hash, note, now_millis(), id.0],
            )
            .map_err(query("recording an operation outcome"))?;

        if changed == 0 {
            return Err(JournalError::UnknownOp(id.0));
        }
        Ok(())
    }

    /// Every operation that was begun and never finished.
    ///
    /// This is crash recovery: on startup these are the operations that were in
    /// flight when the process died.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn incomplete(&self) -> Result<Vec<Op>> {
        self.select(
            "SELECT * FROM ops WHERE status = 'intended' ORDER BY id",
            &[],
            "listing interrupted operations",
        )
    }

    /// Everything that ever happened to `path`, oldest first.
    ///
    /// Matches the path at either end, so a file's history survives being moved.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn history(&self, path: &Path) -> Result<Vec<Op>> {
        let needle = path_str(path);
        self.select(
            "SELECT * FROM ops
             WHERE src_path = ?1 OR dst_path = ?1
                OR (src_root || '/' || src_path) = ?1
                OR (dst_root || '/' || dst_path) = ?1
             ORDER BY id",
            rusqlite::params![needle],
            "reading the history of a path",
        )
    }

    /// Where something ended up, most recent first.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn whereis(&self, locator: &Locator<'_>) -> Result<Vec<Op>> {
        match locator {
            Locator::Hash(hash) => self.select(
                "SELECT * FROM ops WHERE hash = ?1 AND status = 'committed' ORDER BY id DESC",
                rusqlite::params![hash],
                "locating by hash",
            ),
            Locator::Path(path) => {
                let needle = path_str(path);
                self.select(
                    "SELECT * FROM ops
                     WHERE status = 'committed'
                       AND (src_path = ?1 OR dst_path = ?1
                            OR (src_root || '/' || src_path) = ?1
                            OR (dst_root || '/' || dst_path) = ?1)
                     ORDER BY id DESC",
                    rusqlite::params![needle],
                    "locating by path",
                )
            }
        }
    }

    fn select(
        &self,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
        context: &'static str,
    ) -> Result<Vec<Op>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(sql).map_err(query(context))?;
        let rows = stmt
            .query_map(params, row_to_op)
            .map_err(query(context))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(query(context))?;
        Ok(rows)
    }

    /// A poisoned mutex means another thread panicked mid-write. The connection
    /// itself is still sound, and refusing every later write would turn one bug
    /// into a dead journal, so recover the guard rather than propagating.
    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Where the journal lives on this machine.
///
/// # Errors
/// [`JournalError::NoStateDir`] if the platform exposes no standard location.
pub fn default_path() -> Result<PathBuf> {
    let dirs =
        directories::ProjectDirs::from("", "", "tungstate").ok_or(JournalError::NoStateDir)?;
    Ok(dirs.data_dir().join("journal.db"))
}

fn query(context: &'static str) -> impl Fn(rusqlite::Error) -> JournalError {
    move |source| JournalError::Query { context, source }
}

/// Paths are stored as lossy UTF-8.
///
/// A filename that is not valid UTF-8 is rare and, on the platforms tungstate
/// targets, usually a sign of corruption. Storing a lossy form keeps the schema
/// simple and queryable; the alternative is a BLOB column that no human can read
/// in a database dump. Revisit if a real filename ever trips it.
fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// SQLite's INTEGER is signed 64-bit and it has no unsigned type, so sizes are
/// narrowed on the way in. The clamp is unreachable for real files: `i64::MAX`
/// bytes is over nine exabytes.
fn size_to_sql(size: u64) -> i64 {
    i64::try_from(size).unwrap_or(i64::MAX)
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn location(root: Option<String>, path: Option<String>) -> Option<Location> {
    match (root, path) {
        (Some(root), Some(path)) => Some(Location::new(root, path)),
        _ => None,
    }
}

fn row_to_op(row: &rusqlite::Row<'_>) -> rusqlite::Result<Op> {
    let kind_raw: String = row.get("kind")?;
    let status_raw: String = row.get("status")?;
    Ok(Op {
        id: OpId(row.get("id")?),
        // A row whose kind or status we cannot parse came from a newer version of
        // tungstate. Fall back rather than fail, so an old binary can still read.
        kind: OpKind::parse(&kind_raw).unwrap_or(OpKind::Copy),
        status: OpStatus::parse(&status_raw).unwrap_or(OpStatus::Failed),
        source: location(row.get("src_root")?, row.get("src_path")?),
        destination: location(row.get("dst_root")?, row.get("dst_path")?),
        // A negative size means a corrupt row; report it as unknown rather
        // than as zero, which would read as a real empty file.
        size: row
            .get::<_, Option<i64>>("size")?
            .and_then(|v| u64::try_from(v).ok()),
        hash: row.get("hash")?,
        link: row.get("link")?,
        note: row.get("note")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
    })
}

#[cfg(test)]
mod tests;
