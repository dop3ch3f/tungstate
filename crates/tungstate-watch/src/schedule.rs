//! When to come back to a folder, and which events to ignore.
//!
//! Both halves are pure and hold no handles, which is what lets the awkward
//! parts of a watcher be tested without a filesystem: deadline arithmetic, and
//! the set that stops the app's own writes waking it up again.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

/// Folders waiting to be looked at, and when.
#[derive(Debug, Default)]
pub struct Schedule {
    /// Root to the moment it is worth surveying.
    due: BTreeMap<String, Instant>,
}

impl Schedule {
    /// Something happened in `root`; look at it once `cooldown` has passed.
    ///
    /// A second event pushes the deadline out rather than leaving it where it
    /// was. That is the whole of "wait until it stops changing" at this level:
    /// a file still being written keeps arriving, and the folder keeps being
    /// put off until it stops.
    pub fn stirred(&mut self, root: &str, now: Instant, cooldown: Duration) {
        self.due.insert(root.to_string(), now + cooldown);
    }

    /// Roots whose moment has come, removed as they are handed over.
    #[must_use]
    pub fn ready(&mut self, now: Instant) -> Vec<String> {
        let ready: Vec<String> = self
            .due
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(root, _)| root.clone())
            .collect();
        for root in &ready {
            self.due.remove(root);
        }
        ready
    }

    /// How long to wait before anything is worth doing, if anything is.
    #[must_use]
    pub fn quiet_for(&self, now: Instant) -> Option<Duration> {
        self.due
            .values()
            .min()
            .map(|at| at.saturating_duration_since(now))
    }

    /// Whether anything is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.due.is_empty()
    }
}

/// Paths tungstate itself just wrote.
///
/// Every move the executor makes comes back as an event about a file that just
/// changed, which would mark the folder dirty and buy another survey of it. A
/// tidy converges, so the loop would end on its own; what this saves is the
/// **cost**, which on a large folder is one full survey per action.
#[derive(Debug, Default)]
pub struct Echoes {
    /// Path to the moment it stops being expected.
    expected: BTreeMap<String, Instant>,
}

impl Echoes {
    /// Expect events about these paths for a while.
    pub fn expect<I: IntoIterator<Item = String>>(&mut self, paths: I, until: Instant) {
        for path in paths {
            self.expected.insert(path, until);
        }
    }

    /// Whether this event is one of ours, and therefore not news.
    #[must_use]
    pub fn ours(&self, path: &Path, now: Instant) -> bool {
        let key = path.to_string_lossy();
        self.expected
            .get(key.as_ref())
            .is_some_and(|until| *until > now)
    }

    /// Drop what has aged out, so the set does not grow for ever.
    pub fn forget_old(&mut self, now: Instant) {
        self.expected.retain(|_, until| *until > now);
    }

    /// How many paths are still expected.
    #[must_use]
    pub fn len(&self) -> usize {
        self.expected.len()
    }

    /// Whether nothing is expected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.expected.is_empty()
    }
}

/// Which governed root this path is inside, if any.
///
/// The longest match wins, so a folder governed inside another folder gets its
/// own events rather than its parent's.
#[must_use]
pub fn root_of<'a>(path: &Path, roots: &'a [String]) -> Option<&'a str> {
    roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.len())
        .map(String::as_str)
}

/// Whether this path is one tungstate keeps its own things in.
///
/// Events about the policy file and the set-aside area are not news about the
/// folder's contents, and treating them as such makes writing a plan into
/// `.tungstate/` wake the folder that plan is about. The same goes for the
/// files every survey writes to learn what the filesystem can do: heeding
/// those wakes the folder once a second for as long as it is watched.
#[must_use]
pub fn is_ours(path: &Path, root: &str) -> bool {
    let Ok(inside) = path.strip_prefix(root) else {
        return false;
    };
    inside.components().any(|part| {
        let name = part.as_os_str().to_string_lossy();
        tungstate_core::snapshot::RESERVED.contains(&name.as_ref())
            || name.starts_with(tungstate_backend::local::PROBE_PREFIX)
    })
}
