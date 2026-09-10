//! Transfer links: a named source, destination, and the rules for moving
//! between them.
//!
//! Stored here rather than in a config file because a link has to survive a
//! restart for a drain to be resumable, and its definition and its progress are
//! then queryable together.

use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// A configured transfer link.
#[derive(Debug, Clone)]
pub struct Link {
    /// Assigned when the link is created.
    pub id: LinkId,
    /// The name used on the command line.
    pub name: String,
    /// Root of the backend files come from.
    pub source_root: PathBuf,
    /// Root of the backend files go to.
    pub destination_root: PathBuf,
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
    /// Root of the backend files come from.
    pub source_root: PathBuf,
    /// Root of the backend files go to.
    pub destination_root: PathBuf,
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
                 ordering, on_conflict, cooldown_secs, created_at, saved
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                link.name,
                path_str(&link.source_root),
                path_str(&link.destination_root),
                link.source_policy.as_str(),
                link.verify.as_str(),
                link.order.as_str(),
                link.on_conflict.as_str(),
                i64::try_from(link.cooldown.as_secs()).unwrap_or(i64::MAX),
                now_millis(),
                i64::from(link.saved),
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
        source_root: PathBuf::from(text("source_root")?),
        destination_root: PathBuf::from(text("dest_root")?),
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
