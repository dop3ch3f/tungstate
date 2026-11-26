//! Every file of the transfer running now, kept in Rust beside the events.
//!
//! The window keeps the same ledger from those events, but a phone pauses a
//! web view in the background and events sent meanwhile can go unseen. This
//! copy is what the window asks for when it is shown again
//! (`transfer_ledger`), and what a phone's notification counts from. The
//! rules match `state/useTransfer.ts` so the two never disagree about a file.

use std::collections::HashMap;

use serde::Serialize;

/// One file, as `useTransfer`'s `Row` holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LedgerRow {
    pub path: String,
    pub size: u64,
    /// `waiting`, `live`, `checking`, or the engine's outcome word.
    pub state: String,
    pub detail: Option<String>,
    pub done: u64,
    pub checked: u64,
}

/// How far the transfer is, for a notification.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub files: usize,
    /// Files with an outcome, whatever it was.
    pub settled: usize,
    pub bytes: u64,
    pub total: u64,
    /// The file most recently started.
    pub current: Option<String>,
}

#[derive(Debug, Default)]
pub struct Ledger {
    job: Option<u64>,
    rows: Vec<LedgerRow>,
    /// Row indexes by path. An exchange can list one name twice, once per
    /// direction, hence a list. Without it a 10,000-photo drain scanned the
    /// rows three times a file.
    at: HashMap<String, Vec<usize>>,
    current: Option<String>,
    /// Since when the run has been waiting for a connection to answer, in
    /// milliseconds since the epoch.
    waiting: Option<i64>,
}

const MOVING: [&str; 2] = ["live", "checking"];

impl Ledger {
    /// A transfer starts, or starts over after waiting: a fresh ledger.
    pub fn begin(&mut self, job: u64) {
        *self = Self {
            job: Some(job),
            ..Self::default()
        };
    }

    /// The transfer has ended: its rows, handed over, and an empty ledger.
    pub fn finish(&mut self) -> Vec<LedgerRow> {
        std::mem::take(self).rows
    }

    pub fn job(&self) -> Option<u64> {
        self.job
    }

    pub fn rows(&self) -> &[LedgerRow] {
        &self.rows
    }

    pub fn waiting(&self) -> Option<i64> {
        self.waiting
    }

    pub fn wait(&mut self, since: Option<i64>) {
        self.waiting = since;
    }

    fn push(&mut self, row: LedgerRow) {
        self.at
            .entry(row.path.clone())
            .or_default()
            .push(self.rows.len());
        self.rows.push(row);
    }

    /// The row a file event is about: the one in an expected state if the
    /// name is listed twice, else the first.
    fn find(&mut self, path: &str, states: &[&str]) -> Option<&mut LedgerRow> {
        let same = self.at.get(path)?;
        let index = same
            .iter()
            .copied()
            .find(|&i| states.contains(&self.rows[i].state.as_str()))
            .or_else(|| same.first().copied())?;
        self.rows.get_mut(index)
    }

    /// Added to, not replaced: an exchange's second leg plans after the first.
    pub fn planned(&mut self, files: impl IntoIterator<Item = (String, u64)>) {
        for (path, size) in files {
            self.push(LedgerRow {
                path,
                size,
                state: "waiting".into(),
                detail: None,
                done: 0,
                checked: 0,
            });
        }
    }

    pub fn starting(&mut self, path: &str, size: u64) {
        self.current = Some(path.to_string());
        if let Some(row) = self.find(path, &["waiting"]) {
            row.state = "live".into();
            row.done = 0;
            row.checked = 0;
        } else {
            // Not in the plan: a conflict landed it under another name.
            self.push(LedgerRow {
                path: path.to_string(),
                size,
                state: "live".into(),
                detail: None,
                done: 0,
                checked: 0,
            });
        }
    }

    pub fn advanced(&mut self, path: &str, done: u64, total: u64) {
        if let Some(row) = self.find(path, &MOVING) {
            // Back to copying: a file compared and now sending loses "checking".
            row.state = "live".into();
            row.checked = 0;
            row.done = done;
            if total > 0 {
                row.size = total;
            }
        }
    }

    pub fn checking(&mut self, path: &str, done: u64, total: u64) {
        if let Some(row) = self.find(path, &MOVING) {
            row.state = "checking".into();
            row.done = done;
            row.checked = total;
        }
    }

    pub fn finished(&mut self, path: &str, outcome: &str, detail: Option<&str>) {
        let Some(&index) = self.at.get(path).and_then(|same| {
            same.iter()
                .find(|&&i| MOVING.contains(&self.rows[i].state.as_str()))
        }) else {
            return;
        };
        let row = &mut self.rows[index];
        row.state = outcome.to_string();
        row.detail = detail.map(str::to_string);
        row.done = row.size;
    }

    /// What a notification says: computed when asked, at most about once a
    /// second, so the ledger keeps no running sums to get wrong.
    pub fn counts(&self) -> Counts {
        let mut counts = Counts {
            files: self.rows.len(),
            current: self.current.clone(),
            ..Counts::default()
        };
        for row in &self.rows {
            counts.total += row.size;
            match row.state.as_str() {
                "waiting" => {}
                "live" | "checking" => counts.bytes += row.done.min(row.size),
                _ => {
                    counts.settled += 1;
                    counts.bytes += row.size;
                }
            }
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger() -> Ledger {
        let mut ledger = Ledger::default();
        ledger.begin(7);
        ledger.planned([("a.jpg".to_string(), 100), ("b.jpg".to_string(), 300)]);
        ledger
    }

    fn state(ledger: &Ledger, path: &str) -> String {
        ledger
            .rows()
            .iter()
            .find(|r| r.path == path)
            .unwrap()
            .state
            .clone()
    }

    #[test]
    fn a_file_goes_waiting_live_checking_done() {
        let mut l = ledger();
        assert_eq!(state(&l, "a.jpg"), "waiting");
        l.starting("a.jpg", 100);
        l.advanced("a.jpg", 40, 100);
        assert_eq!(state(&l, "a.jpg"), "live");
        l.checking("a.jpg", 100, 50);
        assert_eq!(state(&l, "a.jpg"), "checking");
        l.finished("a.jpg", "transferred", None);
        assert_eq!(state(&l, "a.jpg"), "transferred");
        assert_eq!(l.rows()[0].done, 100);
    }

    #[test]
    fn counts_add_what_is_settled_and_what_is_in_flight() {
        let mut l = ledger();
        l.starting("a.jpg", 100);
        l.finished("a.jpg", "transferred", None);
        l.starting("b.jpg", 300);
        l.advanced("b.jpg", 150, 300);
        let c = l.counts();
        assert_eq!((c.files, c.settled, c.bytes, c.total), (2, 1, 250, 400));
        assert_eq!(c.current.as_deref(), Some("b.jpg"));
    }

    #[test]
    fn a_file_not_in_the_plan_is_shown_rather_than_dropped() {
        let mut l = ledger();
        l.starting("a (1).jpg", 100);
        assert_eq!(state(&l, "a (1).jpg"), "live");
        assert_eq!(l.rows().len(), 3);
    }

    #[test]
    fn a_finish_for_a_file_not_moving_changes_nothing() {
        let mut l = ledger();
        l.finished("a.jpg", "transferred", None);
        assert_eq!(state(&l, "a.jpg"), "waiting");
    }

    #[test]
    fn starting_over_after_a_wait_empties_the_ledger() {
        let mut l = ledger();
        l.wait(Some(5));
        l.begin(7);
        assert_eq!(l.rows(), &[] as &[LedgerRow]);
        assert_eq!(l.waiting(), None);
        assert_eq!(l.job(), Some(7));
    }

    #[test]
    fn finishing_hands_over_the_rows_and_empties_the_ledger() {
        let mut l = ledger();
        let rows = l.finish();
        assert_eq!(rows.len(), 2);
        assert_eq!(l.job(), None);
        assert_eq!(l.counts(), Counts::default());
    }
}
