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
    // v3: a one-off transfer from the browser still needs a link row, so it
    // gets journaling and resume for free, but it must not clutter the list of
    // pairs the user deliberately saved.
    "ALTER TABLE links ADD COLUMN saved INTEGER NOT NULL DEFAULT 1;",
    // v4: a link end stops being a bare path and becomes a place plus a path
    // inside it. Additive throughout: NULL means the local filesystem, so every
    // row written before this migration means exactly what it meant before.
    //
    // No secret column exists here by construction. A password lives in the
    // machine's keychain keyed by connection name, never in this file.
    "CREATE TABLE connections (
         id         INTEGER PRIMARY KEY,
         name       TEXT    NOT NULL UNIQUE,
         scheme     TEXT    NOT NULL,
         host       TEXT,
         port       INTEGER,
         username   TEXT,
         root       TEXT    NOT NULL DEFAULT '',
         options    TEXT    NOT NULL DEFAULT '{}',
         created_at INTEGER NOT NULL
     );

     ALTER TABLE links ADD COLUMN source_connection INTEGER REFERENCES connections (id);
     ALTER TABLE links ADD COLUMN dest_connection   INTEGER REFERENCES connections (id);
     ALTER TABLE ops   ADD COLUMN src_connection    INTEGER REFERENCES connections (id);
     ALTER TABLE ops   ADD COLUMN dst_connection    INTEGER REFERENCES connections (id);",
    // v5: what a link was actually asked to move.
    //
    // Until now a selection existed only as a Vec passed to a worker thread,
    // so it died with the process. That made an interrupted browser transfer
    // unresumable as itself: with nothing to consult, a resumed run walked the
    // whole source root and carried on past what the user had chosen.
    //
    // No rows for a link means the whole root, which is what a saved
    // folder-pair means and what every link written before this already meant.
    // Additive, so nothing needs backfilling.
    "CREATE TABLE link_files (
         link_id INTEGER NOT NULL REFERENCES links (id) ON DELETE CASCADE,
         path    TEXT    NOT NULL,
         PRIMARY KEY (link_id, path)
     ) WITHOUT ROWID;",
    // v6: a link can be put away without being destroyed.
    //
    // An op refers to its link by id, so deleting a link that has ever run
    // would leave `log` and `whereis` naming an id nothing can resolve — the
    // history would survive and stop making sense, which is worse than not
    // being able to delete at all.
    //
    // `deleted_at` rather than reusing `saved`: `saved` already answers a
    // different question (a named pair, or the one-off a browser transfer
    // makes), and a column that answers two questions answers neither. NULL
    // means live, which is what every existing row is without backfilling.
    "ALTER TABLE links ADD COLUMN deleted_at INTEGER;",
    // v7: a reorganisation is a thing that happened, not just a pile of
    // operations that happen to share a moment.
    //
    // Three questions need the row and none is answerable from a bare id on
    // the operations: what were the last few reorganisations (`undo --last`),
    // which folder was this (the operations only know paths), and has this one
    // already been taken back. `undone_at` is NULL until it is, the same
    // tombstone shape as `links.deleted_at`.
    //
    // `snapshot` is the fingerprint the plan was built from, so a saved
    // plan.json applied later can be refused when the folder has moved on.
    //
    // Additive: `plan_id IS NULL` on every existing row, which reads as "a
    // drain did this, not a reorganisation", and is exactly true.
    //
    // `undoes` is what stops "undo the last thing" from undoing the undo and
    // quietly redoing the reorganisation. An undo is itself a plan -- it has
    // to be, or its operations would not show up in `log` -- so without this
    // column the two are indistinguishable from the outside.
    "CREATE TABLE plans (
         id         INTEGER PRIMARY KEY,
         folder     TEXT    NOT NULL,
         snapshot   TEXT    NOT NULL,
         applied_at INTEGER NOT NULL,
         undone_at  INTEGER,
         undoes     INTEGER REFERENCES plans (id)
     );

     ALTER TABLE ops ADD COLUMN plan_id INTEGER REFERENCES plans (id);

     CREATE INDEX ops_plan ON ops (plan_id) WHERE plan_id IS NOT NULL;",
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

    // WAL still allows only one writer at a time, and without this a second
    // process's write returns SQLITE_BUSY the instant it collides rather than
    // waiting for the first to commit. Two drains running at once is an
    // ordinary thing to want — one to the NAS, one somewhere else — and a
    // spurious "database is locked" partway through either of them is not an
    // error anyone can act on.
    //
    // Five seconds because journal writes are two small statements per file,
    // so a real wait is milliseconds; this is long enough to cover a stalled
    // fsync and short enough that a genuinely stuck writer still surfaces.
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(open_err)?;

    // NORMAL skips an fsync per commit, which matters when a drain commits tens
    // of thousands of times. The trade is that a power cut can lose the last few
    // journal writes. Slice 3 orders its work so that costs a redundant re-copy
    // rather than lost data: destination fsynced, then journal, then source
    // removed. A lost "committed" write means we copy the file again.
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(open_err)?;

    // Enforce declared foreign keys. v4 relies on this: a connection with
    // links pointing at it cannot be deleted out from under them.
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(open_err)?;

    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(open_err)?;

    // Reversal of earlier policy, deliberate. Until v4 a newer file was merely
    // skipped, on the principle that an old binary should still be able to read
    // one. That was safe while every stored path was a local absolute path. It
    // is not any more: a v4 link end can be a path relative to a remote
    // connection this build knows nothing about, and an old binary would read
    // it as a local path and drain into the wrong place. Refuse instead.
    let known = i64::try_from(MIGRATIONS.len()).unwrap_or(i64::MAX);
    if current > known {
        return Err(JournalError::TooNew {
            found: current,
            known,
        });
    }

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
