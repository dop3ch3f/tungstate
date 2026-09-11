//! Asking a run to stop, at two different strengths.
//!
//! One flag cannot express two promises. "Stop after these files" undertakes
//! that nothing in flight is abandoned, so it can only be honoured between
//! files. "Stop now" undertakes the opposite — that the run ends within a
//! chunk — and the cost of that is a partly-written file at the destination.
//!
//! Both are safe, and for the same reason: the journal row stays `Intended`
//! and the source is never touched until a copy is verified, so an abandoned
//! file is exactly the shape an interrupted process leaves. Recovery already
//! knows what to do with it.

use std::sync::atomic::{AtomicU8, Ordering};

/// How urgently a run has been asked to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Halt {
    /// Carry on.
    No,
    /// Finish what is in flight, start nothing new.
    AfterThisFile,
    /// Put it down now, wherever it is.
    Now,
}

/// A stop request shared between the window and the run.
///
/// `AtomicU8` rather than a `Mutex<Halt>`: this is read on every chunk of
/// every file in flight, and a lock on that path would be a contention point
/// for the sake of three values that fit in a byte.
#[derive(Debug, Default)]
pub struct Stop(AtomicU8);

impl Stop {
    /// A run nobody has asked to stop.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Finish what is in flight, then stop.
    pub fn after_this_file(&self) {
        self.raise(Halt::AfterThisFile);
    }

    /// Stop within a chunk, abandoning whatever is being written.
    pub fn now(&self) {
        self.raise(Halt::Now);
    }

    /// Back to running. For reuse between runs, not for changing one's mind
    /// mid-run: a worker that has already abandoned a file cannot un-abandon
    /// it.
    pub fn clear(&self) {
        self.0.store(Halt::No as u8, Ordering::Relaxed);
    }

    /// What has been asked for.
    #[must_use]
    pub fn level(&self) -> Halt {
        match self.0.load(Ordering::Relaxed) {
            0 => Halt::No,
            1 => Halt::AfterThisFile,
            _ => Halt::Now,
        }
    }

    /// Whether the run should stop between files. True for either strength.
    #[must_use]
    pub fn asked(&self) -> bool {
        self.level() > Halt::No
    }

    /// Whether the run should drop what it is holding.
    #[must_use]
    pub fn immediate(&self) -> bool {
        self.level() == Halt::Now
    }

    /// Only ever escalates. Someone who pressed "stop now" after "stop after
    /// these files" meant the stronger one, and the race between two clicks
    /// should not be able to resolve to the weaker.
    fn raise(&self, level: Halt) {
        self.0.fetch_max(level as u8, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_stop_is_not_asking_for_anything() {
        let stop = Stop::new();
        assert_eq!(stop.level(), Halt::No);
        assert!(!stop.asked());
        assert!(!stop.immediate());
    }

    #[test]
    fn the_stronger_request_wins_whichever_order_they_arrive_in() {
        let stop = Stop::new();
        stop.now();
        stop.after_this_file();
        assert_eq!(
            stop.level(),
            Halt::Now,
            "a gentle ask must not undo a hard one"
        );

        let other = Stop::new();
        other.after_this_file();
        other.now();
        assert_eq!(other.level(), Halt::Now);
    }

    #[test]
    fn a_gentle_stop_is_asked_for_but_not_immediate() {
        let stop = Stop::new();
        stop.after_this_file();
        assert!(stop.asked());
        assert!(!stop.immediate());
    }
}
