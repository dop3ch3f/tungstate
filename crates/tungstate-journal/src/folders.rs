//! Governed folders: the roots this machine has been asked to look after.
//!
//! The command line does not need this. `tungstate plan` finds its folder by
//! walking up from wherever it was typed, which is the right answer for a
//! command someone runs *inside* a folder. A window has no working directory
//! to walk up from, so it has to be able to show you the folders you have.
//!
//! The root is the identity. Adding the same folder twice is one folder, which
//! is why the root is `UNIQUE` and the name is not — a name is for reading.

use crate::{Journal, JournalError, Result, now_millis, query};

/// Identifies one governed folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FolderId(pub i64);

/// A root this machine looks after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// Identifier.
    pub id: FolderId,
    /// The absolute path, which is the folder's identity.
    pub root: String,
    /// What to call it on screen. Defaults to the last path segment.
    pub name: String,
    /// When it was added, in milliseconds since the Unix epoch.
    pub added_at: i64,
}

impl Journal {
    /// Start looking after `root`.
    ///
    /// # Errors
    /// [`JournalError::DuplicateFolder`] if that root is already governed, or
    /// [`JournalError::Query`] if the row cannot be written.
    pub fn add_folder(&self, root: &str, name: &str) -> Result<FolderId> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO folders (root, name, added_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![root, name, now_millis()],
        )
        .map_err(|error| match error {
            rusqlite::Error::SqliteFailure(e, _)
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                JournalError::DuplicateFolder(root.to_string())
            }
            other => JournalError::Query {
                context: "adding a folder",
                source: other,
            },
        })?;
        Ok(FolderId(conn.last_insert_rowid()))
    }

    /// Every governed folder, oldest first.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn folders(&self) -> Result<Vec<Folder>> {
        let conn = self.lock();
        let mut statement = conn
            .prepare("SELECT id, root, name, added_at FROM folders ORDER BY id")
            .map_err(query("listing folders"))?;
        let rows = statement
            .query_map([], row_to_folder)
            .map_err(query("listing folders"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(query("listing folders"))
    }

    /// One governed folder by its root.
    ///
    /// # Errors
    /// [`JournalError::UnknownFolder`] if that root is not governed.
    pub fn folder_by_root(&self, root: &str) -> Result<Folder> {
        let conn = self.lock();
        conn.query_row(
            "SELECT id, root, name, added_at FROM folders WHERE root = ?1",
            rusqlite::params![root],
            row_to_folder,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => JournalError::UnknownFolder(root.to_string()),
            other => JournalError::Query {
                context: "reading a folder",
                source: other,
            },
        })
    }

    /// Stop looking after `root`.
    ///
    /// The folder's history is untouched: the reorganisations applied to it
    /// stay in the journal, so `log` and `undo --plan` keep working on a folder
    /// that is no longer governed. Forgetting a folder is forgetting to *watch*
    /// it, not forgetting what was done to it.
    ///
    /// # Errors
    /// [`JournalError::UnknownFolder`] if that root is not governed.
    pub fn remove_folder(&self, root: &str) -> Result<()> {
        let conn = self.lock();
        let removed = conn
            .execute(
                "DELETE FROM folders WHERE root = ?1",
                rusqlite::params![root],
            )
            .map_err(query("forgetting a folder"))?;
        if removed == 0 {
            return Err(JournalError::UnknownFolder(root.to_string()));
        }
        Ok(())
    }
}

fn row_to_folder(row: &rusqlite::Row<'_>) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: FolderId(row.get("id")?),
        root: row.get("root")?,
        name: row.get("name")?,
        added_at: row.get("added_at")?,
    })
}
