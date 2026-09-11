//! Transfer links: a named source, destination, and the rules for moving
//! between them.
//!
//! Stored here rather than in a config file because a link has to survive a
//! restart for a drain to be resumable, and its definition and its progress are
//! then queryable together.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::connections::{ConnectionId, Endpoint};
use crate::{Journal, JournalError, Result, now_millis, path_str, query};

/// Identifier for a configured link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LinkId(pub i64);

/// What happens to the original once a transfer is verified.
///
/// Deliberately has no `Default`. Guessing either fails to reclaim space or
/// deletes something the user wanted kept, so the CLI requires an explicit choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePolicy {
    /// Delete it. This is what reclaims space.
    Delete,
    /// Send it to the operating system's trash, recoverable until emptied.
    Trash,
    /// Leave it alone; the link behaves as a verified mirror.
    Keep,
}

/// How thoroughly a transfer is checked before the source is touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerifyLevel {
    /// Compare sizes only. Catches truncation, nothing else.
    Size,
    /// Trust the digest computed while streaming, and check the destination size.
    #[default]
    Hash,
    /// Re-read the destination and compare digests. The only level that proves
    /// the bytes on the far disk are the bytes that were sent.
    Readback,
}

/// The order files are transferred in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    /// Biggest first, so space comes back soonest. The point of a drain.
    #[default]
    LargestFirst,
    /// Smallest first, so the file count drops fastest.
    SmallestFirst,
    /// Oldest modification time first.
    OldestFirst,
    /// Whatever order the walk found them in.
    Discovered,
}

/// What to do when a differently-sized or differently-hashed file already holds
/// the destination name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictAction {
    /// Land the incoming file under a suffixed name.
    Rename,
    /// Leave the source in place and move on.
    Skip,
    /// The incoming file wins; the existing one is quarantined first.
    Replace,
    /// Park the incoming file under `.tungstate-quarantine/` at the destination.
    #[default]
    Quarantine,
}

/// One link's unfinished work.
#[derive(Debug, Clone)]
pub struct Interrupted {
    /// The link the work belongs to, saved or not.
    pub link: Link,
    /// Its operations that were begun and never finished.
    pub ops: Vec<crate::Op>,
}

impl Interrupted {
    /// Total size of the files involved, where it was recorded.
    ///
    /// The size of each *file*, not of the bytes already copied: how much of a
    /// partial actually landed is only knowable by asking the destination, and
    /// that is a network round trip this is not worth doing at startup.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.ops.iter().filter_map(|op| op.size).sum()
    }
}

/// A configured transfer link.
#[derive(Debug, Clone)]
pub struct Link {
    /// Assigned when the link is created.
    pub id: LinkId,
    /// The name used on the command line.
    pub name: String,
    /// Where files come from: which place, and where inside it.
    pub source: Endpoint,
    /// Where files go: which place, and where inside it.
    pub destination: Endpoint,
    /// What happens to originals.
    pub source_policy: SourcePolicy,
    /// How thoroughly transfers are checked.
    pub verify: VerifyLevel,
    /// The order files are taken in.
    pub order: Order,
    /// What an unattended run does with a conflict.
    pub on_conflict: ConflictAction,
    /// How recently a file may have been written and still be skipped.
    pub cooldown: Duration,
    /// False for a one-off transfer started from the browser.
    pub saved: bool,
}

/// The fields needed to create a link.
#[derive(Debug, Clone)]
pub struct NewLink {
    /// The name it will be referred to by. Must be unique.
    pub name: String,
    /// Where files come from: which place, and where inside it.
    pub source: Endpoint,
    /// Where files go: which place, and where inside it.
    pub destination: Endpoint,
    /// What happens to originals.
    pub source_policy: SourcePolicy,
    /// How thoroughly transfers are checked.
    pub verify: VerifyLevel,
    /// The order files are taken in.
    pub order: Order,
    /// What an unattended run does with a conflict.
    pub on_conflict: ConflictAction,
    /// How recently a file may have been written and still be skipped.
    pub cooldown: Duration,
    /// False for a one-off transfer started from the browser.
    pub saved: bool,
}

macro_rules! string_enum {
    ($type:ty { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $type {
            /// How this value is stored, and how it is written on the command line.
            #[must_use]
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }

            /// Parse from storage or from a command-line argument.
            #[must_use]
            pub fn parse(raw: &str) -> Option<Self> {
                match raw { $($text => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

// The same spellings serve the database and the CLI, so a stored value is
// always a value the user could have typed, and vice versa.
string_enum!(SourcePolicy { Delete => "delete", Trash => "trash", Keep => "keep" });
string_enum!(VerifyLevel { Size => "size", Hash => "hash", Readback => "readback" });
string_enum!(Order {
    LargestFirst => "largest-first",
    SmallestFirst => "smallest-first",
    OldestFirst => "oldest-first",
    Discovered => "discovered",
});
string_enum!(ConflictAction {
    Rename => "rename",
    Skip => "skip",
    Replace => "replace",
    Quarantine => "quarantine",
});

impl Journal {
    /// Create a link.
    ///
    /// # Errors
    /// [`JournalError::DuplicateLink`] if the name is taken, or
    /// [`JournalError::Query`] if the row cannot be written.
    pub fn create_link(&self, link: &NewLink) -> Result<LinkId> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO links (
                 name, source_root, dest_root, source_policy, verify,
                 ordering, on_conflict, cooldown_secs, created_at, saved,
                 source_connection, dest_connection
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                link.name,
                path_str(&link.source.path),
                path_str(&link.destination.path),
                link.source_policy.as_str(),
                link.verify.as_str(),
                link.order.as_str(),
                link.on_conflict.as_str(),
                i64::try_from(link.cooldown.as_secs()).unwrap_or(i64::MAX),
                now_millis(),
                i64::from(link.saved),
                link.source.connection.map(|c| c.0),
                link.destination.connection.map(|c| c.0),
            ],
        )
        .map_err(|error| match error {
            rusqlite::Error::SqliteFailure(inner, _)
                if inner.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                JournalError::DuplicateLink(link.name.clone())
            }
            other => JournalError::Query {
                context: "creating a link",
                source: other,
            },
        })?;
        Ok(LinkId(conn.last_insert_rowid()))
    }

    /// Look a link up by the name the user typed.
    ///
    /// # Errors
    /// [`JournalError::UnknownLink`] if there is no such link, or
    /// [`JournalError::Query`] if the row cannot be read.
    pub fn link_by_name(&self, name: &str) -> Result<Link> {
        let conn = self.lock();
        conn.query_row(
            "SELECT * FROM links WHERE name = ?1",
            rusqlite::params![name],
            row_to_link,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => JournalError::UnknownLink(name.to_string()),
            other => JournalError::Query {
                context: "looking up a link",
                source: other,
            },
        })
    }

    /// The pairs the user deliberately saved, in creation order.
    ///
    /// One-off browser transfers are excluded; they exist only so an ad-hoc
    /// move is journaled and resumable like any other.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn links(&self) -> Result<Vec<Link>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT * FROM links WHERE saved = 1 ORDER BY id")
            .map_err(query("listing links"))?;
        let links = stmt
            .query_map([], row_to_link)
            .map_err(query("listing links"))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(query("listing links"))?;
        Ok(links)
    }

    /// Record what a link was asked to move, replacing anything already there.
    ///
    /// Paths are relative to the link's source. An empty list clears the
    /// selection, which restores the "whole source root" meaning rather than
    /// meaning "move nothing" — there is no way to express the latter and no
    /// use for one.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be written.
    pub fn set_files(&self, link: LinkId, paths: &[PathBuf]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn
            .transaction()
            .map_err(query("recording a link's selection"))?;
        tx.execute(
            "DELETE FROM link_files WHERE link_id = ?1",
            rusqlite::params![link.0],
        )
        .map_err(query("clearing a link's selection"))?;
        {
            let mut insert = tx
                // The primary key makes a repeated path a no-op rather than a
                // second copy of the same file in the queue.
                .prepare("INSERT OR IGNORE INTO link_files (link_id, path) VALUES (?1, ?2)")
                .map_err(query("recording a link's selection"))?;
            for path in paths {
                insert
                    .execute(rusqlite::params![link.0, path_str(path)])
                    .map_err(query("recording a link's selection"))?;
            }
        }
        tx.commit().map_err(query("recording a link's selection"))
    }

    /// What a link was asked to move. Empty means the whole source root.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn files_for(&self, link: LinkId) -> Result<Vec<PathBuf>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT path FROM link_files WHERE link_id = ?1 ORDER BY path")
            .map_err(query("reading a link's selection"))?;
        let paths = stmt
            .query_map(rusqlite::params![link.0], |row| {
                row.get::<_, String>(0).map(PathBuf::from)
            })
            .map_err(query("reading a link's selection"))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(query("reading a link's selection"))?;
        Ok(paths)
    }

    /// Look a link up by id.
    ///
    /// # Errors
    /// [`JournalError::UnknownLink`] if there is no such link, or
    /// [`JournalError::Query`] if the row cannot be read.
    pub fn link_by_id(&self, id: LinkId) -> Result<Link> {
        let conn = self.lock();
        conn.query_row(
            "SELECT * FROM links WHERE id = ?1",
            rusqlite::params![id.0],
            row_to_link,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => JournalError::UnknownLink(format!("#{}", id.0)),
            other => JournalError::Query {
                context: "looking up a link by id",
                source: other,
            },
        })
    }

    /// Every link with work begun and never finished, and that work.
    ///
    /// Deliberately not filtered by `saved`. That flag answers "did the user
    /// ask to keep this pair?", which is the right question for the saved-pairs
    /// list and the wrong one here: a one-off transfer from the browser is
    /// unsaved, and it is exactly the case that would otherwise strand a
    /// part-copied file with nothing able to find it again.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn interrupted(&self) -> Result<Vec<Interrupted>> {
        let mut by_link: std::collections::BTreeMap<i64, Vec<crate::Op>> =
            std::collections::BTreeMap::new();
        for op in self.incomplete()? {
            // An op with no link predates links, or was written by something
            // that is not a transfer. There is nothing to resume it with.
            if let Some(id) = op.link_id {
                by_link.entry(id.0).or_default().push(op);
            }
        }

        let mut runs = Vec::with_capacity(by_link.len());
        for (id, ops) in by_link {
            // Propagated rather than skipped. The foreign key makes a missing
            // link unreachable, so if it ever happens something is wrong that
            // is worth hearing about, not worth hiding by returning a shorter
            // list than the truth.
            runs.push(Interrupted {
                link: self.link_by_id(LinkId(id))?,
                ops,
            });
        }
        Ok(runs)
    }

    /// Operations for one link that were begun and never finished.
    ///
    /// This is what a resumed drain reads to find its own interrupted work,
    /// rather than every interrupted operation on the machine.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn incomplete_for_link(&self, link: LinkId) -> Result<Vec<crate::Op>> {
        self.select(
            "SELECT * FROM ops WHERE link_id = ?1 AND status = 'intended' ORDER BY id",
            rusqlite::params![link.0],
            "listing a link's interrupted operations",
        )
    }
}

fn row_to_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<Link> {
    let text = |name: &str| -> rusqlite::Result<String> { row.get(name) };
    Ok(Link {
        id: LinkId(row.get("id")?),
        name: text("name")?,
        source: Endpoint {
            connection: row
                .get::<_, Option<i64>>("source_connection")?
                .map(ConnectionId),
            path: PathBuf::from(text("source_root")?),
        },
        destination: Endpoint {
            connection: row
                .get::<_, Option<i64>>("dest_connection")?
                .map(ConnectionId),
            path: PathBuf::from(text("dest_root")?),
        },
        // An unparseable value means a newer tungstate wrote it. Fall back to the
        // safest reading rather than refusing to load the link at all.
        source_policy: SourcePolicy::parse(&text("source_policy")?).unwrap_or(SourcePolicy::Keep),
        verify: VerifyLevel::parse(&text("verify")?).unwrap_or_default(),
        order: Order::parse(&text("ordering")?).unwrap_or_default(),
        on_conflict: ConflictAction::parse(&text("on_conflict")?).unwrap_or_default(),
        cooldown: Duration::from_secs(
            u64::try_from(row.get::<_, i64>("cooldown_secs")?).unwrap_or(30),
        ),
        saved: row.get::<_, i64>("saved")? != 0,
    })
}

/// The path a link's temporary file takes while in flight.
///
/// Derived from the operation id so a crashed run's leftovers can be found and
/// removed knowing only the journal.
#[must_use]
pub fn temp_name(destination: &Path, op: crate::OpId) -> PathBuf {
    let mut name = destination.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tungstate-{}.part", op.0));
    destination.with_file_name(name)
}
