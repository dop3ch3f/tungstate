//! Schema setup and migrations.
//!
//! Versioning uses SQLite's own `user_version` header field, which is an integer
//! stored in the database file for exactly this purpose. Each migration is
//! applied in order and bumps it. No migration framework for a schema this size.

use rusqlite::Connection;

use crate::{JournalError, Result};

/// Migrations in order. The index plus one is the `user_version` each produces.
const MIGRATIONS: &[&str] = &[
    // v1: the operations table.
    "CREATE TABLE ops (
         id          INTEGER PRIMARY KEY,
         kind        TEXT    NOT NULL,
         status      TEXT    NOT NULL,
         src_root    TEXT,
         src_path    TEXT,
         dst_root    TEXT,
         dst_path    TEXT,
         size        INTEGER,
         hash        TEXT,
         link        TEXT,
         note        TEXT,
         started_at  INTEGER NOT NULL,
         finished_at INTEGER
     );

     -- The four questions anyone asks: what happened to this path, where did
     -- this content go, what was interrupted, and what happened recently.
     CREATE INDEX ops_src_path ON ops (src_path);
     CREATE INDEX ops_dst_path ON ops (dst_path);
     CREATE INDEX ops_hash     ON ops (hash) WHERE hash IS NOT NULL;
     CREATE INDEX ops_status   ON ops (status) WHERE status = 'intended';
     CREATE INDEX ops_started  ON ops (started_at);",
    // v2: transfer links. A link is user intent that has to survive a restart,
    // so a drain interrupted by a closing lid can be resumed by name.
    "CREATE TABLE links (
         id             INTEGER PRIMARY KEY,
         name           TEXT    NOT NULL UNIQUE,
         source_root    TEXT    NOT NULL,
         dest_root      TEXT    NOT NULL,
         source_policy  TEXT    NOT NULL,
         verify         TEXT    NOT NULL,
         ordering       TEXT    NOT NULL,
         on_conflict    TEXT    NOT NULL,
         cooldown_secs  INTEGER NOT NULL,
         created_at     INTEGER NOT NULL
     );

     -- Ops gain a link reference so a resumed run can find just its own
     -- interrupted work rather than every interrupted op on the machine.
     ALTER TABLE ops ADD COLUMN link_id INTEGER REFERENCES links (id);
     CREATE INDEX ops_link ON ops (link_id, status);",
];

/// Bring `conn` up to the current schema, creating it if the file is new.
///
/// # Errors
/// [`JournalError::Open`] if the pragmas or migrations cannot be applied.
pub(crate) fn prepare(conn: &Connection) -> Result<()> {
    configure(conn)?;
    migrate(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    // WAL lets readers run while a write is in flight, and survives a crash
    // mid-transaction. An in-memory database has no WAL, so ignore that failure.
    let _ = conn.pragma_update(None, "journal_mode", "WAL");

    // NORMAL skips an fsync per commit, which matters when a drain commits tens
    // of thousands of times. The trade is that a power cut can lose the last few
    // journal writes. Slice 3 orders its work so that costs a redundant re-copy
    // rather than lost data: destination fsynced, then journal, then source
    // removed. A lost "committed" write means we copy the file again.
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(open_err)?;

    // Enforce declared foreign keys, for the tables slices 6 and 7 will add.
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(open_err)?;

    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(open_err)?;

    let applied = usize::try_from(current).unwrap_or(0);
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(applied) {
        // Each migration and its version bump land together, so an interrupted
        // upgrade cannot leave the file claiming a version it does not have.
        let tx = conn.unchecked_transaction().map_err(open_err)?;
        tx.execute_batch(migration).map_err(open_err)?;
        tx.pragma_update(
            None,
            "user_version",
            i64::try_from(index + 1).unwrap_or(i64::MAX),
        )
        .map_err(open_err)?;
        tx.commit().map_err(open_err)?;
    }
    Ok(())
}

fn open_err(source: rusqlite::Error) -> JournalError {
    JournalError::Open {
        path: std::path::PathBuf::from("<open journal>"),
        source,
    }
}
