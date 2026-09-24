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

// `links` first: it defines the `string_enum!` macro, and `macro_rules!` is
// only in scope for modules declared after it.
#[macro_use]
pub mod links;
pub mod connections;
pub mod ends;
pub mod folders;
pub mod plans;
mod schema;
mod storage;

pub use connections::{
    Connection, ConnectionId, ConnectionSettings, Endpoint, NewConnection, Scheme,
};
pub use ends::{
    EndError, connection_prefix, describe, join_display, parent_display, parse_end, place,
};
pub use folders::{Folder, FolderId};
pub use links::{
    ConflictAction, Link, LinkId, NewLink, Order, Removal, SourcePolicy, VerifyLevel, temp_name,
};
pub use plans::{AppliedPlan, PastPlan, PlanId, Purpose};
pub use storage::{Archive, ArchiveSummary};

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

// Aliased: in this crate `Connection` now means a tungstate connection, and
// having the two spellings collide would be a trap for every later reader.
use rusqlite::Connection as SqliteConnection;

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

    /// A link name is already taken.
    #[error("a link named `{0}` already exists")]
    DuplicateLink(String),

    /// A caller referred to a link that does not exist.
    #[error("no link named `{0}`")]
    UnknownLink(String),

    /// A caller referred to an operation that is not in the journal.
    #[error("no operation with id {0}")]
    UnknownOp(i64),

    /// A folder with that root is already governed.
    #[error("`{0}` is already a governed folder")]
    DuplicateFolder(String),

    /// A caller referred to a folder that is not governed.
    #[error("`{0}` is not a governed folder")]
    UnknownFolder(String),

    /// A caller referred to a reorganisation that is not in the journal.
    #[error("no plan with id {0}")]
    UnknownPlan(i64),

    /// The reorganisation has already been reversed.
    ///
    /// Refused rather than repeated: undoing an undo is a redo wearing the
    /// wrong name, and a caller that meant that should say so.
    #[error("plan {0} has already been undone")]
    PlanAlreadyUndone(i64),

    /// A plan that cannot be taken back at all: it used the desktop's trash,
    /// and only the desktop can put those files back.
    #[error(
        "plan {0} sent files to the trash, so it cannot be undone from here: \
         open the Trash and use Put Back"
    )]
    PlanIrreversible(i64),

    /// A connection name is already taken.
    #[error("a connection named `{0}` already exists")]
    DuplicateConnection(String),

    /// A caller referred to a connection that does not exist.
    #[error("no connection named `{0}`")]
    UnknownConnection(String),

    /// The journal is in memory, so there is nowhere to keep an archive.
    #[error("this journal is held in memory and has no directory to archive into")]
    NotOnDisk,

    /// There is no archive by that name.
    #[error("no archived journal called `{0}`")]
    NoArchive(String),

    /// An archive could not be written, read or removed.
    #[error("could not work with `{path}`")]
    Archive {
        /// The file in question.
        path: std::path::PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },

    /// A document is not a tungstate export, or is one this build cannot read.
    #[error("{0}")]
    BadExport(String),

    /// An import refuses to merge into a journal that already holds something.
    #[error(
        "this journal is not empty; reset it first, so an import can never \
         leave a half-merged state nobody can reason about"
    )]
    NotEmpty,

    /// The journal cannot be put aside while a run has left work behind.
    ///
    /// Those rows are the only thing that knows a part-copied file exists at
    /// the far end. Forgetting them strands it with nothing able to name,
    /// find or sweep it, which is the failure the whole recovery story exists
    /// to prevent.
    #[error(
        "{operations} operation(s) are unfinished; finish or clear them first, \
         with `tungstate link unfinished` to see them"
    )]
    Unfinished {
        /// How many.
        operations: usize,
    },

    /// A connection cannot be removed while a link still points at it.
    #[error("`{0}` is still used by at least one link; remove those links first")]
    ConnectionInUse(String),

    /// A link cannot be removed while a run has left work behind.
    ///
    /// Refused rather than handled: removing it now would strand a part-copied
    /// file at the destination with nothing left able to name it, which is the
    /// one thing this project's recovery story exists to prevent.
    #[error(
        "`{name}` has {operations} unfinished operation(s); finish them with \
         `tungstate link run {name}` or clear them with `tungstate link discard {name}` first"
    )]
    LinkUnfinished {
        /// The link.
        name: String,
        /// How much it left behind.
        operations: usize,
    },

    /// The journal file was written by a newer tungstate than this one.
    ///
    /// Refused rather than skipped. Before v4 every stored link end was a local
    /// absolute path, so an old binary reading a newer file was harmless. A v4
    /// end can be a path relative to a connection this build knows nothing
    /// about, and reading it as a local path would drain into the wrong place.
    #[error(
        "this journal was written by a newer tungstate (schema v{found}; this build knows v{known})"
    )]
    TooNew {
        /// The `user_version` found in the file.
        found: i64,
        /// The highest `user_version` this build can produce.
        known: i64,
    },
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
    /// Empty directory removed.
    RmDir,
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
            Self::RmDir => "rmdir",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "copy" => Some(Self::Copy),
            "move" => Some(Self::Move),
            "rename" => Some(Self::Rename),
            "remove" => Some(Self::Remove),
            "mkdir" => Some(Self::MkDir),
            "rmdir" => Some(Self::RmDir),
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
    /// Which place this is. `None` is the local filesystem.
    pub connection: Option<ConnectionId>,
    /// Backend root, so two folders with a `videos/` subdirectory stay distinct.
    pub root: PathBuf,
    /// Path relative to that root.
    pub path: PathBuf,
}

impl Location {
    /// A location within `root` on the local filesystem.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, path: impl Into<PathBuf>) -> Self {
        Self {
            connection: None,
            root: root.into(),
            path: path.into(),
        }
    }

    /// A location within one end of a link, carrying that end's place.
    #[must_use]
    pub fn within(end: &Endpoint, path: impl Into<PathBuf>) -> Self {
        Self {
            connection: end.connection,
            root: end.path.clone(),
            path: path.into(),
        }
    }

    /// The full path this location denotes, as a path on this machine.
    ///
    /// Only meaningful for a local location. Use [`Location::display_path`]
    /// when the answer is going in front of a person, because that one knows
    /// a remote's separator is not this machine's.
    #[must_use]
    pub fn full(&self) -> PathBuf {
        self.root.join(&self.path)
    }

    /// How this location reads to a human.
    ///
    /// A remote's separator is `/` whatever this machine happens to use, so
    /// the two halves are joined literally rather than through `Path::join`.
    /// Same rule as `remote_key` in the `OpenDAL` adapter, for the same reason:
    /// `inbox\a.mp4` is not a path the far side has ever heard of, and
    /// someone copying it out of `tungstate whereis` would be copying a name
    /// that does not exist.
    #[must_use]
    pub fn display_path(&self) -> String {
        if self.connection.is_none() {
            return self.full().display().to_string();
        }
        let (root, path) = (
            self.root.display().to_string(),
            self.path.display().to_string(),
        );
        match (root.is_empty(), path.is_empty()) {
            (true, _) => path,
            (false, true) => root,
            (false, false) => format!("{}/{path}", root.trim_end_matches('/')),
        }
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
    /// The link this belongs to, so a resumed drain can find its own work.
    pub link_id: Option<links::LinkId>,
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
    /// Which transfer link asked for this, by name.
    pub link: Option<String>,
    /// Which transfer link asked for this, by id.
    ///
    /// The name is for reading; this is for finding the link again, which is
    /// what makes an interrupted run resumable.
    pub link_id: Option<links::LinkId>,
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
    conn: Mutex<SqliteConnection>,
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
        let conn = SqliteConnection::open(path).map_err(|source| JournalError::Open {
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
        let conn = SqliteConnection::open_in_memory().map_err(|source| JournalError::Open {
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
                 src_connection, dst_connection,
                 size, link, link_id, started_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                op.kind.as_str(),
                OpStatus::Intended.as_str(),
                op.source.as_ref().map(|l| path_str(&l.root)),
                op.source.as_ref().map(|l| path_str(&l.path)),
                op.destination.as_ref().map(|l| path_str(&l.root)),
                op.destination.as_ref().map(|l| path_str(&l.path)),
                op.source.as_ref().and_then(|l| l.connection).map(|c| c.0),
                op.destination
                    .as_ref()
                    .and_then(|l| l.connection)
                    .map(|c| c.0),
                op.size.map(size_to_sql),
                op.link,
                op.link_id.map(|id| id.0),
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
        let sql = format!("SELECT * FROM ops WHERE {} ORDER BY id", matches_path());
        let ask = |needle: String| {
            self.select(
                &sql,
                rusqlite::params![needle],
                "reading the history of a path",
            )
        };
        // Both spellings, because rows are stored in whichever form the caller
        // used when they were written -- `folder add` canonicalises its root
        // and a link does not -- and on macOS `/tmp` and `/var` are symlinks,
        // so one spelling silently matches nothing. Neither form can be
        // preferred, so both are tried; see `resolve_for_lookup`.
        let first = ask(path_needle(path))?;
        if !first.is_empty() {
            return Ok(first);
        }
        let resolved = resolve_for_lookup(path);
        if resolved == path {
            return Ok(first);
        }
        ask(path_needle(&resolved))
    }

    /// The most recent operations, newest first.
    ///
    /// Backs the window's activity view, where the interesting rows are the
    /// ones that just happened.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn recent(&self, limit: u32) -> Result<Vec<Op>> {
        self.select(
            "SELECT * FROM ops ORDER BY id DESC LIMIT ?1",
            rusqlite::params![limit],
            "reading recent activity",
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
                let sql = format!(
                    "SELECT * FROM ops WHERE status = 'committed' AND ({})
                     ORDER BY id DESC",
                    matches_path()
                );
                let ask = |needle: String| {
                    self.select(&sql, rusqlite::params![needle], "locating by path")
                };
                // Both spellings, for the reason `history` gives.
                let first = ask(path_needle(path))?;
                if !first.is_empty() {
                    return Ok(first);
                }
                let resolved = resolve_for_lookup(path);
                if resolved == *path {
                    return Ok(first);
                }
                ask(path_needle(&resolved))
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
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, SqliteConnection> {
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

pub(crate) fn query(context: &'static str) -> impl Fn(rusqlite::Error) -> JournalError {
    move |source| JournalError::Query { context, source }
}

/// Paths are stored as lossy UTF-8.
///
/// A filename that is not valid UTF-8 is rare and, on the platforms tungstate
/// targets, usually a sign of corruption. Storing a lossy form keeps the schema
/// simple and queryable; the alternative is a BLOB column that no human can read
/// in a database dump. Revisit if a real filename ever trips it.
pub(crate) fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A path as a needle for the history queries, with separators normalised.
///
/// Windows only, and deliberately. A journal row keeps its root and its path
/// apart and the queries join them with `/`, so on Windows the stored spelling
/// is `C:\Users\me\dst/a.mp4` while every caller types backslashes
/// throughout. Nothing ever matched, which is why `tungstate log <path>` had
/// never worked there.
///
/// Not done on Unix, where a backslash is an ordinary character in a filename:
/// normalising there would make `a\b.mp4` and `a/b.mp4` compare equal, which
/// is two different files answering to one name.
/// The path to look a file up by.
///
/// A journal row stores the root as it was canonicalised when the folder was
/// governed — `/private/tmp/media` — and somebody asking about it types
/// `/tmp/media`, because on macOS `/tmp` is a symlink. The two are plainly the
/// same file and the lookup missed, silently, answering "no matching
/// operations" for a file with a full history.
///
/// `canonicalize` on its own will not do: the most useful question `whereis`
/// answers is about a file that has *moved*, whose old path no longer exists
/// and cannot be canonicalised at all. So resolve the deepest ancestor that
/// does exist and put the rest of the path back on the end.
#[must_use]
pub fn resolve_for_lookup(path: &Path) -> PathBuf {
    if let Some(real) = canonical(path) {
        return real;
    }
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut here = path.to_path_buf();
    while let Some(parent) = here.parent().map(Path::to_path_buf) {
        let Some(name) = here.file_name().map(std::ffi::OsStr::to_os_string) else {
            break;
        };
        tail.push(name);
        if let Some(real) = canonical(&parent) {
            let mut out = real;
            for part in tail.iter().rev() {
                out.push(part);
            }
            return out;
        }
        here = parent;
    }
    path.to_path_buf()
}

/// `canonicalize`, with Windows's verbatim prefix taken back off.
fn canonical(path: &Path) -> Option<PathBuf> {
    let real = path.canonicalize().ok()?;
    let text = real.to_string_lossy();
    match without_verbatim_prefix(&text) {
        stripped if stripped.len() == text.len() => Some(real.clone()),
        stripped => Some(PathBuf::from(stripped)),
    }
}

/// Windows's `canonicalize` answers with a verbatim path — `\\?\C:\media` —
/// and no journal row is ever spelled that way, so a resolved path carrying
/// the prefix could never match one and the whole fallback would be dead
/// weight on Windows.
///
/// A UNC verbatim path is left alone: `\\?\UNC\server\share` does not mean
/// `UNC\server\share`, and stripping it would change which machine it names.
///
/// Free of `cfg!` so both answers are testable from either platform, as
/// [`separators_as_slashes`] already is.
fn without_verbatim_prefix(text: &str) -> &str {
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with(r"UNC\") => rest,
        _ => text,
    }
}

pub(crate) fn path_needle(path: &Path) -> String {
    separators_as_slashes(&path.to_string_lossy(), cfg!(windows))
}

/// The body of [`path_needle`], with the platform as an argument.
///
/// Taken as a parameter rather than read from `cfg!` so both answers can be
/// tested from either platform. The Windows behaviour is the one that was
/// wrong for the life of the project, and proving it on the machine doing the
/// work beats waiting for a CI run to disagree.
fn separators_as_slashes(text: &str, windows: bool) -> String {
    if windows {
        text.replace('\\', "/")
    } else {
        text.to_string()
    }
}

/// The `WHERE` fragment that matches a path at either end of an operation.
///
/// The concatenation branches are scoped to local rows on purpose.
/// `root || '/' || path` is only a real path when the row is local; for a
/// remote it splices a connection-relative root onto a connection-relative
/// path and can collide with a genuine local file of the same shape.
fn matches_path() -> String {
    path_clause(cfg!(windows))
}

/// The body of [`matches_path`], with the platform as an argument.
fn path_clause(windows: bool) -> String {
    // SQL string literals have no escape character, so `'\'` here is one
    // backslash to SQLite rather than the start of an escape.
    let normalised = |expression: String| {
        if windows {
            format!("replace({expression}, '\\', '/')")
        } else {
            expression
        }
    };
    let joined = |root: &str, path: &str| normalised(format!("{root} || '/' || {path}"));

    format!(
        "{} = ?1 OR {} = ?1
         OR (src_connection IS NULL AND {} = ?1)
         OR (dst_connection IS NULL AND {} = ?1)",
        normalised("src_path".to_string()),
        normalised("dst_path".to_string()),
        joined("src_root", "src_path"),
        joined("dst_root", "dst_path"),
    )
}

/// SQLite's INTEGER is signed 64-bit and it has no unsigned type, so sizes are
/// narrowed on the way in. The clamp is unreachable for real files: `i64::MAX`
/// bytes is over nine exabytes.
fn size_to_sql(size: u64) -> i64 {
    i64::try_from(size).unwrap_or(i64::MAX)
}

pub(crate) fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn location(
    root: Option<String>,
    path: Option<String>,
    connection: Option<i64>,
) -> Option<Location> {
    match (root, path) {
        (Some(root), Some(path)) => Some(Location {
            connection: connection.map(ConnectionId),
            root: PathBuf::from(root),
            path: PathBuf::from(path),
        }),
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
        source: location(
            row.get("src_root")?,
            row.get("src_path")?,
            row.get("src_connection")?,
        ),
        destination: location(
            row.get("dst_root")?,
            row.get("dst_path")?,
            row.get("dst_connection")?,
        ),
        // A negative size means a corrupt row; report it as unknown rather
        // than as zero, which would read as a real empty file.
        size: row
            .get::<_, Option<i64>>("size")?
            .and_then(|v| u64::try_from(v).ok()),
        hash: row.get("hash")?,
        link: row.get("link")?,
        link_id: row.get::<_, Option<i64>>("link_id")?.map(links::LinkId),
        note: row.get("note")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
    })
}

#[cfg(test)]
mod tests;

/// A digest remembered from a previous pass, and what it described.
///
/// Kept so a second duplicate pass over an untouched folder reads nothing.
/// The key is the path; the *check* is size and mtime, because a file edited
/// in place keeps its name and a stale digest is a wrong answer about
/// somebody's files rather than a slow one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Remembered {
    /// Digest of the ends of the file, if one was taken.
    pub partial: Option<String>,
    /// Digest of every byte, if one was taken.
    pub whole: Option<String>,
    /// What it looks or sounds like, if anything ever looked. Carries the name
    /// of the algorithm that produced it, so a print from another version of
    /// the arithmetic is not mistaken for this one's.
    pub print: Option<String>,
}

impl Journal {
    /// What was remembered about a file, if it has not changed since.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the row cannot be read.
    pub fn remembered(
        &self,
        root: &str,
        path: &str,
        size: u64,
        mtime: Option<i64>,
    ) -> Result<Option<Remembered>> {
        let conn = self.lock();
        let found = conn
            .query_row(
                "SELECT partial, whole, print FROM hashes
                 WHERE root = ?1 AND path = ?2 AND size = ?3
                   AND ((mtime IS NULL AND ?4 IS NULL) OR mtime = ?4)",
                rusqlite::params![root, path, i64::try_from(size).unwrap_or(i64::MAX), mtime],
                |row| {
                    Ok(Remembered {
                        partial: row.get("partial")?,
                        whole: row.get("whole")?,
                        print: row.get("print")?,
                    })
                },
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(query("reading a remembered digest")(other)),
            })?;
        Ok(found)
    }

    /// Remember a digest, replacing whatever was there for that path.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the row cannot be written.
    pub fn remember(
        &self,
        root: &str,
        path: &str,
        size: u64,
        mtime: Option<i64>,
        digest: &Remembered,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO hashes (root, path, size, mtime, partial, whole, print, seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (root, path) DO UPDATE SET
                 size = excluded.size,
                 mtime = excluded.mtime,
                 partial = COALESCE(excluded.partial, hashes.partial),
                 whole = COALESCE(excluded.whole, hashes.whole),
                 print = COALESCE(excluded.print, hashes.print),
                 seen_at = excluded.seen_at",
            rusqlite::params![
                root,
                path,
                i64::try_from(size).unwrap_or(i64::MAX),
                mtime,
                digest.partial,
                digest.whole,
                digest.print,
                now_millis()
            ],
        )
        .map_err(query("remembering a digest"))?;
        Ok(())
    }

    /// Forget every digest for a root. Used when a folder is no longer
    /// governed, and by the tests that prove invalidation works.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be removed.
    pub fn forget_digests(&self, root: &str) -> Result<usize> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM hashes WHERE root = ?1",
            rusqlite::params![root],
        )
        .map_err(query("forgetting digests"))
    }
}

impl Journal {
    /// An answer remembered from a previous run, if there is one.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the row cannot be read.
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.lock();
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            rusqlite::params![key],
            |row| row.get::<_, String>("value"),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(query("reading a setting")(other)),
        })
    }

    /// Remember an answer, replacing any previous one.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the row cannot be written.
    pub fn remember_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO settings (key, value, set_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value, set_at = excluded.set_at",
            rusqlite::params![key, value, now_millis()],
        )
        .map_err(query("remembering a setting"))?;
        Ok(())
    }
}

impl Journal {
    /// A content hash this journal already knows for a file, if it can be
    /// trusted for the file as it is now.
    ///
    /// Every verified transfer records the BLAKE3 of what it wrote, so for
    /// anything tungstate put somewhere the digest is already here and costs
    /// no reading at all. Trusted only when the size still matches and the
    /// file has not been written since we recorded it: `unchanged_since` is
    /// the file's own mtime, and an op that finished after that describes the
    /// bytes that are there now.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn hash_at(
        &self,
        root: &str,
        path: &str,
        size: u64,
        unchanged_since: Option<i64>,
    ) -> Result<Option<String>> {
        // Matched on the path and the size in SQL, then on the root in Rust,
        // because the two spellings of one folder cannot be compared without
        // resolving them and SQLite cannot follow a symlink. A link writes the
        // root it was given and `folder add` canonicalises, so on macOS one
        // side says `/tmp` and the other `/private/tmp`.
        let here = resolve_for_lookup(Path::new(root));
        let conn = self.lock();
        let mut statement = conn
            .prepare(
                "SELECT dst_root, hash, finished_at FROM ops
                 WHERE dst_path = ?1 AND size = ?2
                   AND hash IS NOT NULL AND status = 'committed'
                   AND finished_at IS NOT NULL
                 ORDER BY id DESC LIMIT 32",
            )
            .map_err(query("reading a recorded hash"))?;
        let rows = statement
            .query_map(
                rusqlite::params![path, i64::try_from(size).unwrap_or(i64::MAX)],
                |row| {
                    Ok((
                        row.get::<_, String>("dst_root")?,
                        row.get::<_, String>("hash")?,
                        row.get::<_, i64>("finished_at")?,
                    ))
                },
            )
            .map_err(query("reading a recorded hash"))?;

        for row in rows {
            let (stored, hash, finished) = row.map_err(query("reading a recorded hash"))?;
            if resolve_for_lookup(Path::new(&stored)) != here {
                continue;
            }
            // Written at or after the file's own mtime, so the digest
            // describes the bytes that are there now rather than bytes that
            // have since been overwritten.
            if unchanged_since.is_none_or(|since| finished >= since) {
                return Ok(Some(hash));
            }
        }
        Ok(None)
    }
}
