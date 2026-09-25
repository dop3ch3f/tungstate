//! Reorganisations: what was applied to a folder, and whether it was taken back.
//!
//! An operation already knows its two paths. What it cannot know is which
//! *run* it belonged to, which folder that run was about, or whether that run
//! has since been reversed — and `undo --last 3` needs all three. So a plan is
//! a row, and an operation points at it.

use crate::{Journal, JournalError, OpId, Result, now_millis, query};

/// Identifies one applied reorganisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PlanId(pub i64);

/// What a reorganisation was for, which is which window lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// A folder filed by its rules.
    Tidy,
    /// Extra copies set aside or trashed.
    Duplicates,
    /// A run of a sync, which may copy to several members.
    Sync,
}

impl Purpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tidy => "tidy",
            Self::Duplicates => "duplicates",
            Self::Sync => "sync",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "tidy" => Some(Self::Tidy),
            "duplicates" => Some(Self::Duplicates),
            "sync" => Some(Self::Sync),
            _ => None,
        }
    }
}

/// A past reorganisation with what it amounted to, for a history list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastPlan {
    /// The plan itself.
    pub plan: AppliedPlan,
    /// Files it moved, set aside or trashed. Not directories, and not ops
    /// that failed or were skipped.
    pub files: u64,
    /// Bytes in those files.
    pub bytes: u64,
}

/// One reorganisation as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedPlan {
    /// Identifier, which is what `undo --plan` takes.
    pub id: PlanId,
    /// The folder root it was applied to.
    pub folder: String,
    /// The snapshot fingerprint the plan was built from.
    pub snapshot: String,
    /// When it was applied, in milliseconds since the Unix epoch.
    pub applied_at: i64,
    /// When it was taken back, if it has been.
    pub undone_at: Option<i64>,
    /// Whether this can be taken back at all.
    ///
    /// False for a duplicate pass told to use the desktop's trash: those files
    /// are the operating system's to restore, and a half-working undo would be
    /// worse than a refusal.
    pub reversible: bool,
    /// What it was for. `None` for a plan written before v11, or an undo.
    pub purpose: Option<Purpose>,
    /// The reorganisation this one reverses, if it is an undo.
    ///
    /// An undo is itself a plan, because its operations have to show up in
    /// `log` like everything else. This is what keeps "undo the last thing"
    /// from picking one up and quietly redoing the work.
    pub undoes: Option<PlanId>,
}

impl AppliedPlan {
    /// Whether this has already been reversed.
    #[must_use]
    pub fn is_undone(&self) -> bool {
        self.undone_at.is_some()
    }

    /// Whether this is itself an undo, rather than a reorganisation.
    #[must_use]
    pub fn is_an_undo(&self) -> bool {
        self.undoes.is_some()
    }

    /// Whether `undo --last` should offer this one.
    #[must_use]
    pub fn is_undoable(&self) -> bool {
        self.reversible && !self.is_undone() && !self.is_an_undo()
    }
}

impl Journal {
    /// Record that a reorganisation is starting.
    ///
    /// Written before the first operation, for the same reason an operation's
    /// intent is written before the filesystem is touched: a crash halfway
    /// through must leave something that names what was being attempted.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the row cannot be written.
    pub fn begin_plan(
        &self,
        folder: &str,
        snapshot: &str,
        undoes: Option<PlanId>,
    ) -> Result<PlanId> {
        self.begin_plan_that(folder, snapshot, undoes, true)
    }

    /// Record a reorganisation, saying whether it can ever be taken back.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the row cannot be written.
    pub fn begin_plan_that(
        &self,
        folder: &str,
        snapshot: &str,
        undoes: Option<PlanId>,
        reversible: bool,
    ) -> Result<PlanId> {
        self.begin_plan_for(folder, snapshot, undoes, reversible, None)
    }

    /// Record a reorganisation, saying what it is for.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the row cannot be written.
    pub fn begin_plan_for(
        &self,
        folder: &str,
        snapshot: &str,
        undoes: Option<PlanId>,
        reversible: bool,
        purpose: Option<Purpose>,
    ) -> Result<PlanId> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO plans (folder, snapshot, applied_at, undoes, reversible, purpose)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                folder,
                snapshot,
                now_millis(),
                undoes.map(|p| p.0),
                i64::from(reversible),
                purpose.map(Purpose::as_str)
            ],
        )
        .map_err(query("recording a plan"))?;
        Ok(PlanId(conn.last_insert_rowid()))
    }

    /// Attach an operation to the plan that asked for it.
    ///
    /// Separate from [`Journal::begin`] rather than a field on `NewOp`, because
    /// `NewOp` is the drain's vocabulary and a drain has no plan. One optional
    /// field that is always `None` on one of two callers is a field that
    /// invites being forgotten by the other.
    ///
    /// # Errors
    /// [`JournalError::UnknownOp`] if `op` was never begun, or
    /// [`JournalError::Query`] if the update fails.
    pub fn attach_to_plan(&self, op: OpId, plan: PlanId) -> Result<()> {
        let conn = self.lock();
        let changed = conn
            .execute(
                "UPDATE ops SET plan_id = ?1 WHERE id = ?2",
                rusqlite::params![plan.0, op.0],
            )
            .map_err(query("attaching an operation to its plan"))?;
        if changed == 0 {
            return Err(JournalError::UnknownOp(op.0));
        }
        Ok(())
    }

    /// One reorganisation by id.
    ///
    /// # Errors
    /// [`JournalError::UnknownPlan`] if there is no such plan.
    pub fn plan_by_id(&self, id: PlanId) -> Result<AppliedPlan> {
        let conn = self.lock();
        conn.query_row(
            "SELECT id, folder, snapshot, applied_at, undone_at, undoes, reversible, purpose
             FROM plans WHERE id = ?1",
            rusqlite::params![id.0],
            row_to_plan,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => JournalError::UnknownPlan(id.0),
            other => JournalError::Query {
                context: "reading a plan",
                source: other,
            },
        })
    }

    /// The most recent reorganisations, newest first.
    ///
    /// Includes ones already undone, because "undo the last three" has to be
    /// able to say *that one is already back* rather than silently counting
    /// past it.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn recent_plans(&self, limit: usize) -> Result<Vec<AppliedPlan>> {
        let conn = self.lock();
        let mut statement = conn
            .prepare(
                "SELECT id, folder, snapshot, applied_at, undone_at, undoes, reversible, purpose
                 FROM plans ORDER BY id DESC LIMIT ?1",
            )
            .map_err(query("listing recent plans"))?;
        let rows = statement
            .query_map(
                rusqlite::params![i64::try_from(limit).unwrap_or(i64::MAX)],
                row_to_plan,
            )
            .map_err(query("listing recent plans"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(query("listing recent plans"))
    }

    /// The most recent plans for one folder, newest first.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn recent_plans_for(&self, folder: &str, limit: usize) -> Result<Vec<AppliedPlan>> {
        let conn = self.lock();
        let mut statement = conn
            .prepare(
                "SELECT id, folder, snapshot, applied_at, undone_at, undoes, reversible, purpose
                 FROM plans WHERE folder = ?1 ORDER BY id DESC LIMIT ?2",
            )
            .map_err(query("listing a folder's plans"))?;
        let rows = statement
            .query_map(
                rusqlite::params![folder, i64::try_from(limit).unwrap_or(i64::MAX)],
                row_to_plan,
            )
            .map_err(query("listing a folder's plans"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(query("listing a folder's plans"))
    }

    /// Past reorganisations for one purpose, newest first, with what each
    /// amounted to. Undos are left out: they show as the plan they undid,
    /// marked put back.
    ///
    /// A plan from before v11 has no purpose written down, so it is judged by
    /// what it did: every file it touched went to the set-aside area or the
    /// trash means duplicates, anything else a tidy. One aggregate query, so a
    /// long history costs one pass over the plan's ops through `ops_plan`.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn past_plans(&self, purpose: Purpose, limit: usize) -> Result<Vec<PastPlan>> {
        let conn = self.lock();
        let mut statement = conn
            .prepare(
                "SELECT p.id, p.folder, p.snapshot, p.applied_at, p.undone_at, p.undoes,
                        p.reversible, p.purpose,
                        COUNT(o.id) AS files,
                        COALESCE(SUM(o.size), 0) AS bytes,
                        COALESCE(SUM(o.kind = 'remove'
                                     OR instr(o.dst_path, '.tungstate-quarantine') > 0), 0) AS aside
                 FROM plans p
                 LEFT JOIN ops o ON o.plan_id = p.id
                      AND o.kind IN ('rename', 'remove') AND o.status = 'committed'
                 WHERE p.undoes IS NULL
                 GROUP BY p.id
                 HAVING COALESCE(p.purpose,
                          CASE WHEN COUNT(o.id) > 0 AND aside = COUNT(o.id)
                               THEN 'duplicates' ELSE 'tidy' END) = ?1
                 ORDER BY p.id DESC LIMIT ?2",
            )
            .map_err(query("listing past plans"))?;
        let rows = statement
            .query_map(
                rusqlite::params![purpose.as_str(), i64::try_from(limit).unwrap_or(i64::MAX)],
                |row| {
                    Ok(PastPlan {
                        plan: row_to_plan(row)?,
                        files: u64::try_from(row.get::<_, i64>("files")?).unwrap_or(0),
                        bytes: u64::try_from(row.get::<_, i64>("bytes")?).unwrap_or(0),
                    })
                },
            )
            .map_err(query("listing past plans"))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(query("listing past plans"))
    }

    /// Everything one reorganisation did, oldest first.
    ///
    /// Oldest first is the order it was applied in, so reversing it is what
    /// undo wants and `.rev()` is the whole of the difference.
    ///
    /// # Errors
    /// [`JournalError::Query`] if the rows cannot be read.
    pub fn ops_for_plan(&self, plan: PlanId) -> Result<Vec<crate::Op>> {
        self.select(
            "SELECT * FROM ops WHERE plan_id = ?1 ORDER BY id",
            rusqlite::params![plan.0],
            "reading a plan's operations",
        )
    }

    /// Mark a reorganisation as taken back.
    ///
    /// # Errors
    /// [`JournalError::UnknownPlan`] if there is no such plan, or
    /// [`JournalError::PlanAlreadyUndone`] if it has already been reversed —
    /// refused rather than repeated, because undoing an undo is a redo wearing
    /// the wrong name.
    pub fn mark_undone(&self, plan: PlanId) -> Result<()> {
        let existing = self.plan_by_id(plan)?;
        if existing.is_undone() {
            return Err(JournalError::PlanAlreadyUndone(plan.0));
        }
        let conn = self.lock();
        conn.execute(
            "UPDATE plans SET undone_at = ?1 WHERE id = ?2",
            rusqlite::params![now_millis(), plan.0],
        )
        .map_err(query("marking a plan undone"))?;
        Ok(())
    }
}

fn row_to_plan(row: &rusqlite::Row<'_>) -> rusqlite::Result<AppliedPlan> {
    Ok(AppliedPlan {
        id: PlanId(row.get("id")?),
        folder: row.get("folder")?,
        snapshot: row.get("snapshot")?,
        applied_at: row.get("applied_at")?,
        undone_at: row.get("undone_at")?,
        undoes: row.get::<_, Option<i64>>("undoes")?.map(PlanId),
        reversible: row.get::<_, i64>("reversible")? != 0,
        purpose: row
            .get::<_, Option<String>>("purpose")?
            .as_deref()
            .and_then(Purpose::parse),
    })
}
