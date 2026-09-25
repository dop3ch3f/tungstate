//! Syncs: sets of folders kept in step, their members, and what each member
//! held after the last run.
//!
//! The baseline is **append-only**. A run writes rows and never updates one;
//! the current reading of a path is the latest row whose plan has not been
//! undone. That is what lets `undo` retire a whole run's memory by marking its
//! plan, with no second log to keep in step.

use std::time::Duration;

use rusqlite::OptionalExtension as _;

use crate::connections::ConnectionId;
use crate::links::{ConflictAction, VerifyLevel};
use crate::plans::PlanId;
use crate::{Journal, JournalError, Result, now_millis, query};

/// Identifies a sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SyncId(pub i64);

/// Identifies one member of a sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemberId(pub i64);

/// Which members send and which receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncDirection {
    Push,
    Pull,
    All,
}

/// How hard a first meeting between two copies is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FirstCheck {
    #[default]
    Full,
    Sampled,
    Size,
}

/// What removing a copy means, where a sync removes anything at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnRemove {
    #[default]
    SetAside,
    Delete,
}

string_enum!(SyncDirection { Push => "push", Pull => "pull", All => "all" });
string_enum!(FirstCheck { Full => "full", Sampled => "sampled", Size => "size" });
string_enum!(OnRemove { SetAside => "set-aside", Delete => "delete" });

/// One member as it is to be added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMember {
    /// A word a person chose, used in every sentence about it.
    pub name: String,
    /// `None` for a folder on this machine.
    pub connection: Option<ConnectionId>,
    pub path: String,
}

/// A sync as it is to be created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSync {
    pub name: String,
    pub direction: SyncDirection,
    pub exact: bool,
    /// The member's name, for `Push` and `Pull`.
    pub anchor: Option<String>,
    pub on_conflict: ConflictAction,
    pub on_remove: OnRemove,
    pub verify: VerifyLevel,
    pub cooldown: Duration,
    pub first_check: FirstCheck,
}

/// One member, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub id: MemberId,
    pub ordinal: i64,
    pub name: String,
    pub connection: Option<ConnectionId>,
    pub path: String,
}

/// A sync, as stored, with its members in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sync {
    pub id: SyncId,
    pub name: String,
    pub direction: SyncDirection,
    pub exact: bool,
    pub anchor: Option<MemberId>,
    pub on_conflict: ConflictAction,
    pub on_remove: OnRemove,
    pub verify: VerifyLevel,
    pub cooldown: Duration,
    pub first_check: FirstCheck,
    pub on_launch: bool,
    pub continuous: bool,
    /// Milliseconds since the epoch. A sync removed and made again under the
    /// same name is a different sync, and its runs start here.
    pub created_at: i64,
    pub members: Vec<Member>,
}

/// The settings of a sync that can change after it is made. The members and
/// the direction cannot: changing either would make the baseline describe a
/// different sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSettings {
    pub exact: bool,
    pub on_conflict: ConflictAction,
    pub on_remove: OnRemove,
    pub verify: VerifyLevel,
    pub cooldown: Duration,
    pub first_check: FirstCheck,
}

/// What a member held at a path, as recorded after a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub member: MemberId,
    pub path: String,
    /// False for "absent, by decision".
    pub present: bool,
    pub size: u64,
    /// Milliseconds, as that member reported it.
    pub mtime: Option<i64>,
    pub hash: Option<String>,
}

impl Journal {
    /// Create a sync and its members in one go.
    ///
    /// # Errors
    /// [`JournalError::SyncExists`] if the name is taken,
    /// [`JournalError::BadSync`] if the anchor is not one of the members, or
    /// [`JournalError::Query`] if a row cannot be written.
    pub fn create_sync(&self, sync: &NewSync, members: &[NewMember]) -> Result<SyncId> {
        if self.sync_by_name(&sync.name).is_ok() {
            return Err(JournalError::SyncExists(sync.name.clone()));
        }
        if let Some(anchor) = &sync.anchor
            && !members.iter().any(|m| &m.name == anchor)
        {
            return Err(JournalError::BadSync(format!(
                "the anchor `{anchor}` is not one of the members"
            )));
        }
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(query("starting a sync"))?;
        tx.execute(
            "INSERT INTO syncs (name, direction, exact, on_conflict, on_remove, verify,
                                cooldown_secs, first_check, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                sync.name,
                sync.direction.as_str(),
                i64::from(sync.exact),
                sync.on_conflict.as_str(),
                sync.on_remove.as_str(),
                sync.verify.as_str(),
                i64::try_from(sync.cooldown.as_secs()).unwrap_or(i64::MAX),
                sync.first_check.as_str(),
                now_millis(),
            ],
        )
        .map_err(query("creating a sync"))?;
        let id = tx.last_insert_rowid();
        for (ordinal, member) in members.iter().enumerate() {
            tx.execute(
                "INSERT INTO sync_members (sync_id, ordinal, name, connection, path)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    id,
                    i64::try_from(ordinal).unwrap_or(i64::MAX),
                    member.name,
                    member.connection.map(|c| c.0),
                    member.path,
                ],
            )
            .map_err(query("adding a member"))?;
            if sync.anchor.as_deref() == Some(member.name.as_str()) {
                let member_id = tx.last_insert_rowid();
                tx.execute(
                    "UPDATE syncs SET anchor = ?1 WHERE id = ?2",
                    rusqlite::params![member_id, id],
                )
                .map_err(query("setting the anchor"))?;
            }
        }
        tx.commit().map_err(query("creating a sync"))?;
        Ok(SyncId(id))
    }

    /// A sync by name, with its members.
    ///
    /// # Errors
    /// [`JournalError::UnknownSync`] if there is none by that name.
    pub fn sync_by_name(&self, name: &str) -> Result<Sync> {
        let id: Option<i64> = self
            .lock()
            .query_row(
                "SELECT id FROM syncs WHERE name = ?1 AND deleted_at IS NULL",
                [name],
                |row| row.get(0),
            )
            .optional()
            .map_err(query("finding a sync"))?;
        let id = id.ok_or_else(|| JournalError::UnknownSync(name.to_string()))?;
        self.sync_by_id(SyncId(id))
    }

    /// A sync by id, with its members.
    ///
    /// # Errors
    /// [`JournalError::UnknownSync`] if there is none, or
    /// [`JournalError::Query`] if it cannot be read.
    pub fn sync_by_id(&self, id: SyncId) -> Result<Sync> {
        let conn = self.lock();
        let mut sync = conn
            .query_row("SELECT * FROM syncs WHERE id = ?1", [id.0], row_to_sync)
            .optional()
            .map_err(query("reading a sync"))?
            .ok_or_else(|| JournalError::UnknownSync(id.0.to_string()))?;
        let mut statement = conn
            .prepare("SELECT * FROM sync_members WHERE sync_id = ?1 ORDER BY ordinal, id")
            .map_err(query("reading members"))?;
        sync.members = statement
            .query_map([id.0], row_to_member)
            .map_err(query("reading members"))?
            .collect::<std::result::Result<_, _>>()
            .map_err(query("reading members"))?;
        Ok(sync)
    }

    /// Every sync not removed, in creation order.
    ///
    /// # Errors
    /// [`JournalError::Query`] if they cannot be read.
    pub fn syncs(&self) -> Result<Vec<Sync>> {
        let ids: Vec<i64> = {
            let conn = self.lock();
            let mut statement = conn
                .prepare("SELECT id FROM syncs WHERE deleted_at IS NULL ORDER BY id")
                .map_err(query("listing syncs"))?;
            statement
                .query_map([], |row| row.get(0))
                .map_err(query("listing syncs"))?
                .collect::<std::result::Result<_, _>>()
                .map_err(query("listing syncs"))?
        };
        ids.into_iter()
            .map(|id| self.sync_by_id(SyncId(id)))
            .collect()
    }

    /// Add a member to an existing sync. It is filled on the next run.
    ///
    /// # Errors
    /// [`JournalError::BadSync`] if a member by that name exists, or
    /// [`JournalError::Query`] if the row cannot be written.
    pub fn add_member(&self, sync: SyncId, member: &NewMember) -> Result<MemberId> {
        let existing = self.sync_by_id(sync)?;
        if existing.members.iter().any(|m| m.name == member.name) {
            return Err(JournalError::BadSync(format!(
                "`{}` already has a member called `{}`",
                existing.name, member.name
            )));
        }
        let next = existing
            .members
            .iter()
            .map(|m| m.ordinal)
            .max()
            .map_or(0, |n| n + 1);
        let conn = self.lock();
        conn.execute(
            "INSERT INTO sync_members (sync_id, ordinal, name, connection, path)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                sync.0,
                next,
                member.name,
                member.connection.map(|c| c.0),
                member.path
            ],
        )
        .map_err(query("adding a member"))?;
        Ok(MemberId(conn.last_insert_rowid()))
    }

    /// Take a member out of a sync, with everything remembered about it.
    ///
    /// Its files are not touched: leaving a sync is not deleting anything.
    ///
    /// # Errors
    /// [`JournalError::BadSync`] if it is the anchor or one of the last two.
    pub fn remove_member(&self, sync: SyncId, name: &str) -> Result<()> {
        let existing = self.sync_by_id(sync)?;
        let Some(member) = existing.members.iter().find(|m| m.name == name) else {
            return Err(JournalError::BadSync(format!(
                "`{}` has no member called `{name}`",
                existing.name
            )));
        };
        if existing.anchor == Some(member.id) {
            return Err(JournalError::BadSync(format!(
                "`{name}` is the anchor of `{}`; a push or pull needs one",
                existing.name
            )));
        }
        if existing.members.len() <= 2 {
            return Err(JournalError::BadSync(format!(
                "`{}` would be left with one member, and one folder is not a sync",
                existing.name
            )));
        }
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(query("removing a member"))?;
        tx.execute("DELETE FROM sync_state WHERE member_id = ?1", [member.id.0])
            .map_err(query("forgetting a member"))?;
        tx.execute("DELETE FROM sync_members WHERE id = ?1", [member.id.0])
            .map_err(query("removing a member"))?;
        tx.commit().map_err(query("removing a member"))?;
        Ok(())
    }

    /// Put a sync away. Its files stay where they are.
    ///
    /// A tombstone, as links have, so the plans and operations that name it
    /// still resolve in History.
    ///
    /// # Errors
    /// [`JournalError::BadSync`] if one of its legs has work in flight.
    pub fn remove_sync(&self, sync: SyncId) -> Result<()> {
        let conn = self.lock();
        let in_flight: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM ops o JOIN links l ON o.link_id = l.id
                 WHERE l.sync_id = ?1 AND o.status = 'intended'",
                [sync.0],
                |row| row.get(0),
            )
            .map_err(query("checking for work in flight"))?;
        if in_flight > 0 {
            return Err(JournalError::BadSync(
                "a run was interrupted part-way; run it again to finish before removing".into(),
            ));
        }
        conn.execute(
            "UPDATE syncs SET deleted_at = ?1 WHERE id = ?2",
            rusqlite::params![now_millis(), sync.0],
        )
        .map_err(query("removing a sync"))?;
        Ok(())
    }

    /// Every member's current reading of every path it has held: the latest
    /// row per member and path whose plan has not been undone.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn baseline_for(&self, sync: SyncId) -> Result<Vec<Reading>> {
        let conn = self.lock();
        // The latest surviving row per (member, path). `id` rather than
        // `recorded_at` for "latest", because two rows written in the same
        // millisecond still have an order.
        let mut statement = conn
            .prepare(
                "SELECT s.member_id, s.path, s.present, s.size, s.mtime, s.hash
                 FROM sync_state s
                 JOIN sync_members m ON m.id = s.member_id
                 JOIN plans p ON p.id = s.plan_id
                 WHERE m.sync_id = ?1 AND p.undone_at IS NULL
                   AND s.id = (
                       SELECT MAX(t.id) FROM sync_state t JOIN plans q ON q.id = t.plan_id
                       WHERE t.member_id = s.member_id AND t.path = s.path AND q.undone_at IS NULL)
                 ORDER BY s.member_id, s.path",
            )
            .map_err(query("reading a baseline"))?;
        statement
            .query_map([sync.0], |row| {
                Ok(Reading {
                    member: MemberId(row.get(0)?),
                    path: row.get(1)?,
                    present: row.get::<_, i64>(2)? != 0,
                    size: u64::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                    mtime: row.get(4)?,
                    hash: row.get(5)?,
                })
            })
            .map_err(query("reading a baseline"))?
            .collect::<std::result::Result<_, _>>()
            .map_err(query("reading a baseline"))
    }

    /// Remember what members hold, as of `plan`.
    ///
    /// # Errors
    /// [`JournalError::PlanAlreadyUndone`] if `plan` has been undone: a
    /// reading written under an undone plan would never be current, and
    /// writing it anyway would be a bug somewhere else hidden.
    pub fn record_baseline(&self, readings: &[Reading], plan: PlanId) -> Result<()> {
        if self.plan_by_id(plan)?.is_undone() {
            return Err(JournalError::PlanAlreadyUndone(plan.0));
        }
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(query("recording a baseline"))?;
        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO sync_state (member_id, path, present, size, mtime, hash,
                                             plan_id, recorded_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .map_err(query("recording a baseline"))?;
            let at = now_millis();
            for reading in readings {
                insert
                    .execute(rusqlite::params![
                        reading.member.0,
                        reading.path,
                        i64::from(reading.present),
                        i64::try_from(reading.size).unwrap_or(i64::MAX),
                        reading.mtime,
                        reading.hash,
                        plan.0,
                        at,
                    ])
                    .map_err(query("recording a baseline"))?;
            }
        }
        tx.commit().map_err(query("recording a baseline"))?;
        Ok(())
    }

    /// Change how a sync behaves from its next run on.
    ///
    /// # Errors
    /// [`JournalError::UnknownSync`] if it does not exist, or
    /// [`JournalError::Query`] if the row cannot be written.
    pub fn update_sync(&self, sync: SyncId, settings: &SyncSettings) -> Result<()> {
        let changed = self
            .lock()
            .execute(
                "UPDATE syncs SET exact = ?1, on_conflict = ?2, on_remove = ?3, verify = ?4,
                                  cooldown_secs = ?5, first_check = ?6
                 WHERE id = ?7 AND deleted_at IS NULL",
                rusqlite::params![
                    i64::from(settings.exact),
                    settings.on_conflict.as_str(),
                    settings.on_remove.as_str(),
                    settings.verify.as_str(),
                    i64::try_from(settings.cooldown.as_secs()).unwrap_or(i64::MAX),
                    settings.first_check.as_str(),
                    sync.0,
                ],
            )
            .map_err(query("changing a sync"))?;
        if changed == 0 {
            return Err(JournalError::UnknownSync(sync.0.to_string()));
        }
        Ok(())
    }

    /// A sync's runs, newest first, undone ones included and undos left out.
    ///
    /// Only runs since it was made: a sync removed and made again under the
    /// same name must not be able to undo its predecessor's work.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn sync_runs(&self, sync: &Sync, limit: usize) -> Result<Vec<crate::AppliedPlan>> {
        Ok(self
            .recent_plans_for(&format!("sync:{}", sync.name), limit)?
            .into_iter()
            .filter(|plan| plan.applied_at >= sync.created_at && !plan.is_an_undo())
            .collect())
    }

    /// Every reading a run remembered, oldest first.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn readings_for_plan(&self, plan: PlanId) -> Result<Vec<Reading>> {
        let conn = self.lock();
        let mut statement = conn
            .prepare(
                "SELECT member_id, path, present, size, mtime, hash FROM sync_state
                 WHERE plan_id = ?1 ORDER BY id",
            )
            .map_err(query("reading a run's readings"))?;
        statement
            .query_map([plan.0], |row| {
                Ok(Reading {
                    member: MemberId(row.get(0)?),
                    path: row.get(1)?,
                    present: row.get::<_, i64>(2)? != 0,
                    size: u64::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
                    mtime: row.get(4)?,
                    hash: row.get(5)?,
                })
            })
            .map_err(query("reading a run's readings"))?
            .collect::<std::result::Result<_, _>>()
            .map_err(query("reading a run's readings"))
    }

    /// The files a leg committed since `since`, which is how a run learns what
    /// landed and so what to remember about the member it landed on.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn landed(&self, link: crate::LinkId, since: i64) -> Result<Vec<crate::Op>> {
        self.select(
            "SELECT * FROM ops WHERE link_id = ?1 AND status = 'committed'
                 AND started_at >= ?2 ORDER BY id",
            rusqlite::params![link.0, since],
            "reading what a leg landed",
        )
    }

    /// Create a link a sync owns: one leg, from one member to another.
    ///
    /// Beside `create_link` rather than a new field on `NewLink`, which half a
    /// dozen callers build by hand and none of which is a sync.
    ///
    /// # Errors
    /// As [`Journal::create_link`].
    pub fn create_leg(&self, sync: SyncId, link: &crate::NewLink) -> Result<crate::LinkId> {
        let id = self.create_link(link)?;
        self.lock()
            .execute(
                "UPDATE links SET sync_id = ?1 WHERE id = ?2",
                rusqlite::params![sync.0, id.0],
            )
            .map_err(query("marking a leg"))?;
        Ok(id)
    }
}

fn row_to_sync(row: &rusqlite::Row<'_>) -> rusqlite::Result<Sync> {
    let text = |column: &str| row.get::<_, String>(column);
    let bad = |what: &'static str| {
        move |raw: String| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unknown {what} `{raw}`").into(),
            )
        }
    };
    let direction = text("direction")?;
    let on_conflict = text("on_conflict")?;
    let on_remove = text("on_remove")?;
    let verify = text("verify")?;
    let first_check = text("first_check")?;
    Ok(Sync {
        id: SyncId(row.get("id")?),
        name: row.get("name")?,
        direction: SyncDirection::parse(&direction)
            .ok_or_else(|| bad("direction")(direction.clone()))?,
        exact: row.get::<_, i64>("exact")? != 0,
        anchor: row.get::<_, Option<i64>>("anchor")?.map(MemberId),
        on_conflict: ConflictAction::parse(&on_conflict)
            .ok_or_else(|| bad("conflict action")(on_conflict.clone()))?,
        on_remove: OnRemove::parse(&on_remove).ok_or_else(|| bad("removal")(on_remove.clone()))?,
        verify: VerifyLevel::parse(&verify).ok_or_else(|| bad("verify level")(verify.clone()))?,
        cooldown: Duration::from_secs(
            u64::try_from(row.get::<_, i64>("cooldown_secs")?).unwrap_or(0),
        ),
        first_check: FirstCheck::parse(&first_check)
            .ok_or_else(|| bad("first check")(first_check.clone()))?,
        on_launch: row.get::<_, i64>("on_launch")? != 0,
        continuous: row.get::<_, i64>("continuous")? != 0,
        created_at: row.get("created_at")?,
        members: Vec::new(),
    })
}

fn row_to_member(row: &rusqlite::Row<'_>) -> rusqlite::Result<Member> {
    Ok(Member {
        id: MemberId(row.get("id")?),
        ordinal: row.get("ordinal")?,
        name: row.get("name")?,
        connection: row.get::<_, Option<i64>>("connection")?.map(ConnectionId),
        path: row.get("path")?,
    })
}
