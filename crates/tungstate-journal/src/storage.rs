//! Taking the journal out as a document, and putting one back.
//!
//! A clean slate that destroys the record of where two hundred gigabytes of
//! video went is not a feature, it is a cliff. So a reset writes the journal
//! out first, and every archive stays listable and restorable from inside the
//! app. Nobody has to know there is a database involved.
//!
//! JSON rather than a copy of the database file, which is the more important
//! decision. A `.db` restores byte-for-byte and is useless to anyone without
//! SQLite; a document can be read, scripted against, diffed, and — the reason
//! that matters — carried across a schema change that no migration could span.
//!
//! **The tables are read by reflection, not by a hand-written list of
//! columns.** `SELECT *` plus the statement's own column names means a column
//! added in some later slice is exported and imported without anyone
//! remembering to come back here. The alternative is a struct per table that
//! silently drops whatever it has not been taught about, and the day you find
//! out is the day you needed the archive.

use std::path::{Path, PathBuf};

use rusqlite::Connection as SqliteConnection;
use rusqlite::types::ValueRef;
use serde_json::{Map, Value};

use crate::{Journal, JournalError, Result, now_millis};

/// The document format's own version, which is not the database's.
///
/// Two numbers because they answer different questions. `schema` says what the
/// journal looked like; this says how to read the document describing it. A
/// future tungstate whose schema cannot migrate from an old one can still read
/// an old export, because the document describes itself.
pub const EXPORT_VERSION: u32 = 1;

/// Where archives are kept, beside the journal.
const ARCHIVE_DIR: &str = "archives";

/// Imported and exported in this order: a row may reference one earlier in the
/// list, never one later.
const TABLES: [&str; 8] = [
    "connections",
    "links",
    "link_files",
    "folders",
    "plans",
    "ops",
    "hashes",
    "settings",
];

/// One archived journal, described well enough to choose between.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    /// The file name, which is how it is asked for.
    pub name: String,
    /// When it was written, in milliseconds since the Unix epoch.
    pub archived_at: i64,
    /// Bytes on disk.
    pub size: u64,
    /// What is in it, or `None` if it cannot be read. Reported rather than
    /// hidden: an archive that will not open is worth seeing in the list.
    pub summary: Option<ArchiveSummary>,
}

/// What an archive holds, for someone deciding whether they want it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveSummary {
    /// The document format it was written in.
    pub export_version: u32,
    /// The database schema it came from.
    pub schema_version: u32,
    /// Saved links.
    pub links: usize,
    /// Connections.
    pub connections: usize,
    /// Operations of every status.
    pub operations: usize,
    /// Reorganisations applied to a folder.
    pub plans: usize,
    /// When the earliest operation began, if there is one.
    pub first_op: Option<i64>,
    /// When the latest operation began, if there is one.
    pub last_op: Option<i64>,
}

impl Journal {
    /// Write the whole journal out as a document.
    ///
    /// Deliberately carries no passwords. Those live in the keychain under a
    /// key derived from the connection's name, so a connection restored under
    /// the same name finds its password again without one ever being written
    /// to a file that gets copied around.
    ///
    /// # Errors
    /// [`JournalError::Query`] if a table cannot be read.
    pub fn export(&self) -> Result<Value> {
        let conn = self.lock();
        let mut document = Map::new();
        document.insert("tungstate_export".into(), EXPORT_VERSION.into());
        document.insert("schema_version".into(), schema_version(&conn)?.into());
        document.insert("exported_at".into(), now_millis().into());

        for table in TABLES {
            document.insert(table.to_string(), Value::Array(dump(&conn, table)?));
        }
        Ok(Value::Object(document))
    }

    /// Read a document back in.
    ///
    /// Refuses unless the journal is empty, so there is never a half-merged
    /// state nobody can reason about. Ids are preserved, which is what keeps
    /// an operation pointing at the link it belonged to.
    ///
    /// # Errors
    /// [`JournalError::NotEmpty`] if anything is already here,
    /// [`JournalError::BadExport`] if the document is not one of ours.
    pub fn import(&self, document: &Value) -> Result<usize> {
        let version = document
            .get("tungstate_export")
            .and_then(Value::as_u64)
            .ok_or_else(|| JournalError::BadExport("this is not a tungstate export".into()))?;
        if version > u64::from(EXPORT_VERSION) {
            return Err(JournalError::BadExport(format!(
                "this export is version {version} and this build understands {EXPORT_VERSION}; \
                 use a newer tungstate to read it"
            )));
        }

        let mut conn = self.lock();
        for table in TABLES {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .map_err(crate::query("checking the journal is empty"))?;
            if count > 0 {
                return Err(JournalError::NotEmpty);
            }
        }

        // One transaction for the lot: a half-loaded journal is worse than a
        // refused one, and an import that fails partway should leave nothing.
        let tx = conn
            .transaction()
            .map_err(crate::query("beginning an import"))?;
        let mut loaded = 0;
        for table in TABLES {
            if let Some(rows) = document.get(table).and_then(Value::as_array) {
                loaded += load(&tx, table, rows)?;
            }
        }
        tx.commit().map_err(crate::query("committing an import"))?;
        Ok(loaded)
    }

    /// Write the journal out to an archive and continue with an empty one.
    ///
    /// Returns the archive's name, which is what [`Journal::restore`] takes.
    /// Nothing is destroyed.
    ///
    /// # Errors
    /// [`JournalError::Unfinished`] if a run left work behind, because a
    /// journal that forgets a part-copied file leaves it at the destination
    /// with nothing able to name, find or sweep it.
    pub fn reset(&self) -> Result<String> {
        self.refuse_while_unfinished()?;
        let name = self.archive_now()?;
        self.clear()?;
        Ok(name)
    }

    /// Bring an archive back, putting the current journal aside first.
    ///
    /// Restoring is itself undoable, which is the point: restoring the wrong
    /// one costs nothing.
    ///
    /// # Errors
    /// As [`Journal::reset`], plus [`JournalError::NoArchive`] if there is no
    /// archive by that name.
    pub fn restore(&self, name: &str) -> Result<String> {
        self.refuse_while_unfinished()?;
        let path = self.archive_dir()?.join(safe_name(name)?);
        let raw = std::fs::read_to_string(&path).map_err(|source| JournalError::Archive {
            path: path.clone(),
            source,
        })?;
        let document: Value = serde_json::from_str(&raw)
            .map_err(|e| JournalError::BadExport(format!("`{name}` is not readable JSON: {e}")))?;

        // Parsed before anything is touched, so a corrupt archive cannot cost
        // you the journal you already had.
        let put_aside = self.archive_now()?;
        self.clear()?;
        self.import(&document)?;
        Ok(put_aside)
    }

    /// Every archive, newest first.
    ///
    /// # Errors
    /// [`JournalError::Archive`] if the directory cannot be read.
    pub fn archives(&self) -> Result<Vec<Archive>> {
        let dir = self.archive_dir()?;
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let entries = std::fs::read_dir(&dir).map_err(|source| JournalError::Archive {
            path: dir.clone(),
            source,
        })?;

        let mut found: Vec<Archive> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                // Case-insensitively, because a name that came back from a
                // filesystem that folded its case is still one of ours.
                if !is_json(&name) {
                    return None;
                }
                let meta = entry.metadata().ok();
                Some(Archive {
                    archived_at: modified_millis(meta.as_ref()),
                    size: meta.map_or(0, |m| m.len()),
                    summary: summarise(&entry.path()),
                    name,
                })
            })
            .collect();
        // Newest first: the one you want back is almost always the last
        // one written.
        found.sort_by_key(|archive| std::cmp::Reverse(archive.archived_at));
        Ok(found)
    }

    /// Delete an archive for good.
    ///
    /// The only call here that loses anything, which is why it is separate
    /// from everything else and why it is the only one needing a confirmation.
    ///
    /// # Errors
    /// [`JournalError::NoArchive`] if there is none by that name.
    pub fn forget_archive(&self, name: &str) -> Result<()> {
        let path = self.archive_dir()?.join(safe_name(name)?);
        if !path.is_file() {
            return Err(JournalError::NoArchive(name.to_string()));
        }
        std::fs::remove_file(&path).map_err(|source| JournalError::Archive { path, source })
    }

    /// Where archives live for this journal.
    fn archive_dir(&self) -> Result<PathBuf> {
        let conn = self.lock();
        let path = conn.path().unwrap_or_default().to_string();
        if path.is_empty() || path == ":memory:" {
            return Err(JournalError::NotOnDisk);
        }
        Ok(Path::new(&path)
            .parent()
            .unwrap_or(Path::new("."))
            .join(ARCHIVE_DIR))
    }

    /// Write the current journal out under a timestamped name.
    fn archive_now(&self) -> Result<String> {
        let dir = self.archive_dir()?;
        std::fs::create_dir_all(&dir).map_err(|source| JournalError::Archive {
            path: dir.clone(),
            source,
        })?;

        let document = self.export()?;
        let (name, path) = free_name(&dir, &stamp(now_millis()));
        let text = serde_json::to_string_pretty(&document)
            .map_err(|e| JournalError::BadExport(e.to_string()))?;
        std::fs::write(&path, text).map_err(|source| JournalError::Archive { path, source })?;
        Ok(name)
    }

    /// Empty every table, leaving the schema alone.
    ///
    /// Rows rather than the file: the connection stays open and valid, which
    /// means no dance with SQLite's `-wal` companion and no window in which
    /// the journal does not exist.
    fn clear(&self) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn
            .transaction()
            .map_err(crate::query("beginning a reset"))?;
        // Reverse order, so a row is never deleted while another still
        // references it.
        for table in TABLES.iter().rev() {
            tx.execute(&format!("DELETE FROM {table}"), [])
                .map_err(crate::query("clearing the journal"))?;
        }
        tx.commit().map_err(crate::query("committing a reset"))
    }

    /// Refuse while a run has left work behind.
    ///
    /// The same rule as removing a link or a connection, for the sharpest
    /// version of the same reason: those rows are the only thing that knows a
    /// part-copied file exists at the far end.
    fn refuse_while_unfinished(&self) -> Result<()> {
        let waiting: i64 = self
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM ops WHERE status = 'intended'",
                [],
                |row| row.get(0),
            )
            .map_err(crate::query("counting unfinished work"))?;
        if waiting > 0 {
            return Err(JournalError::Unfinished {
                operations: usize::try_from(waiting).unwrap_or(usize::MAX),
            });
        }
        Ok(())
    }
}

/// Every row of `table`, each as an object keyed by the column names the
/// database itself reports.
fn dump(conn: &SqliteConnection, table: &str) -> Result<Vec<Value>> {
    let mut statement = conn
        .prepare(&format!("SELECT * FROM {table}"))
        .map_err(crate::query("reading a table for export"))?;
    let columns: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect();

    let rows = statement
        .query_map([], |row| {
            let mut object = Map::new();
            for (index, name) in columns.iter().enumerate() {
                object.insert(name.clone(), to_json(row.get_ref(index)?));
            }
            Ok(Value::Object(object))
        })
        .map_err(crate::query("reading a table for export"))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(crate::query("reading a table for export"))
}

/// Insert `rows` into `table`, keeping only the columns this schema has.
///
/// A column the document carries and this build does not know is skipped
/// rather than refused: that is an archive from a *newer* tungstate, and
/// losing one field beats losing the whole restore.
fn load(tx: &rusqlite::Transaction<'_>, table: &str, rows: &[Value]) -> Result<usize> {
    let known: Vec<String> = tx
        .prepare(&format!("SELECT * FROM {table} LIMIT 0"))
        .map_err(crate::query("preparing an import"))?
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect();

    let mut written = 0;
    for row in rows {
        let Some(object) = row.as_object() else {
            return Err(JournalError::BadExport(format!(
                "a row in `{table}` is not an object"
            )));
        };
        let present: Vec<&String> = known.iter().filter(|c| object.contains_key(*c)).collect();
        if present.is_empty() {
            continue;
        }

        let names = present
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let holes = present
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let values: Vec<rusqlite::types::Value> = present
            .iter()
            .map(|c| from_json(&object[c.as_str()]))
            .collect();

        tx.execute(
            &format!("INSERT INTO {table} ({names}) VALUES ({holes})"),
            rusqlite::params_from_iter(values),
        )
        .map_err(crate::query("writing a row from an import"))?;
        written += 1;
    }
    Ok(written)
}

/// A SQLite value as JSON. Blobs are refused rather than mangled; nothing in
/// this schema stores one, and quietly base64-ing a surprise would be worse
/// than saying so.
fn to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => Value::from(n),
        ValueRef::Real(f) => Value::from(f),
        ValueRef::Text(bytes) => Value::from(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => Value::from(format!("<{} bytes of binary>", bytes.len())),
    }
}

/// The inverse. A JSON number that is not an integer becomes a real, which is
/// what SQLite would have stored anyway.
fn from_json(value: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as Sql;
    match value {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .map_or_else(|| Sql::Real(n.as_f64().unwrap_or(0.0)), Sql::Integer),
        Value::String(s) => Sql::Text(s.clone()),
        // An object or an array in a column means the options blob, which is
        // stored as its JSON text.
        other => Sql::Text(other.to_string()),
    }
}

fn schema_version(conn: &SqliteConnection) -> Result<u32> {
    conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map(|v| u32::try_from(v).unwrap_or(0))
        .map_err(crate::query("reading the schema version"))
}

/// What is in an archive, read without loading it.
fn summarise(path: &Path) -> Option<ArchiveSummary> {
    let document: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let count = |table: &str| {
        document
            .get(table)
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    };
    let started: Vec<i64> = document
        .get("ops")
        .and_then(Value::as_array)
        .map(|ops| {
            ops.iter()
                .filter_map(|op| op.get("started_at").and_then(Value::as_i64))
                .collect()
        })
        .unwrap_or_default();

    Some(ArchiveSummary {
        export_version: document
            .get("tungstate_export")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())?,
        schema_version: document
            .get("schema_version")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0),
        links: count("links"),
        connections: count("connections"),
        operations: count("ops"),
        plans: count("plans"),
        first_op: started.iter().min().copied(),
        last_op: started.iter().max().copied(),
    })
}

fn modified_millis(meta: Option<&std::fs::Metadata>) -> i64 {
    meta.and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// An archive name with no path in it.
///
/// The name arrives from a window or a command line, so it is untrusted input
/// about to be joined onto a directory. One with a separator could name any
/// file on the machine, and restore would read it.
fn safe_name(name: &str) -> Result<String> {
    let bare =
        Path::new(name).components().count() == 1 && !name.contains('/') && !name.contains('\\');
    if bare && is_json(name) {
        Ok(name.to_string())
    } else {
        Err(JournalError::NoArchive(name.to_string()))
    }
}

/// A name in `dir` that is not taken, and the path to it.
///
/// The stamp is second-resolution because that is what a person can read, and
/// two archives inside one second is not hypothetical: a restore archives the
/// current journal immediately after reading the one it is restoring, and
/// without this the second write lands on the first. Overwriting an archive
/// would be the one way this feature could lose something.
fn free_name(dir: &Path, stamp: &str) -> (String, PathBuf) {
    let mut name = format!("{stamp}.json");
    let mut nth = 2;
    while dir.join(&name).exists() {
        name = format!("{stamp}-{nth}.json");
        nth += 1;
    }
    let path = dir.join(&name);
    (name, path)
}

/// Whether a name is one of our documents, ignoring case.
fn is_json(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

/// `2026-09-14T16-10-42`. Colons are illegal in a filename on Windows and
/// awkward everywhere else, so the time is separated with dashes.
fn stamp(millis: i64) -> String {
    let seconds = millis.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}-{:02}-{:02}",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Days since the Unix epoch as a calendar date, in UTC.
///
/// Howard Hinnant's civil-from-days, which is the standard way to do this
/// without a date library: it shifts the year to start in March so the leap
/// day lands at the end and the month-length arithmetic becomes a formula
/// rather than a table. Written out because the journal has no date
/// dependency and one filename does not justify adding one.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (if month <= 2 { year + 1 } else { year }, month, day)
}
