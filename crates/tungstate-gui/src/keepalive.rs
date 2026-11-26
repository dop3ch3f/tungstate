//! Keeping a phone's transfer going with the screen off or the app in the
//! background, and showing it in the notification shade.
//!
//! A guard: [`Keepalive::begin`] when a run the person started begins, and
//! dropping it ends it, so a run that panics still lets the phone sleep and
//! takes its notification away. The work itself stays on the run's own
//! thread; the phone's side only keeps the process alive and shows how far.
//! Fed from Rust rather than the window because a phone pauses a background
//! web view's scripts, which is exactly when the notification matters.
//!
//! On desktop it does nothing: a desktop app is not suspended behind a lock.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::AppHandle;
use tauri_plugin_photos::Notice;
use tungstate_transfer::Stop;

use crate::ledger::Counts;

/// How often the notification is updated. Android rate-limits updates that
/// come faster, and the Stop button is asked about on the same beat.
const BEAT: Duration = Duration::from_secs(1);

/// What a run is, in the notification's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Errand {
    /// "Copying to Area51", "Moving to Area51", "Freeing up space on Area51".
    pub title: String,
    /// Named when waiting for it: "Waiting for Area51 to answer".
    pub far_side: String,
}

impl Errand {
    /// `far_side` as the engine describes an end: `Docker:` for a
    /// connection's top, `Docker:Photos/2026` inside it, a path for a folder
    /// here. Said as the window says it: `Docker`, `Docker › Photos › 2026`.
    pub fn transfer(removes_originals: bool, far_side: &str) -> Self {
        let verb = if removes_originals {
            "Moving"
        } else {
            "Copying"
        };
        let far_side = match far_side.split_once(':') {
            Some((name, inside)) if !name.contains('/') => std::iter::once(name)
                .chain(inside.split('/').filter(|part| !part.is_empty()))
                .collect::<Vec<_>>()
                .join(" › "),
            _ => far_side
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or(far_side)
                .to_string(),
        };
        Self {
            title: format!("{verb} to {far_side}"),
            far_side,
        }
    }
}

/// The notification for `counts`: plain words, and a bar in thousandths.
pub fn notice(errand: &Errand, counts: &Counts, waiting: bool) -> Notice {
    let text = if waiting {
        format!(
            "Waiting for {} to answer. It carries on when the connection is back.",
            errand.far_side
        )
    } else if counts.files == 0 {
        "Getting ready".to_string()
    } else {
        let mut text = format!("{} of {}", counts.settled, counts.files);
        if let Some(current) = counts
            .current
            .as_deref()
            .filter(|_| counts.settled < counts.files)
        {
            let name = current.rsplit('/').next().unwrap_or(current);
            text.push_str(" · ");
            text.push_str(name);
        }
        text
    };
    let permille = if counts.total == 0 {
        0
    } else {
        // u128 so a multi-terabyte run cannot overflow on the way.
        u32::try_from(u128::from(counts.bytes) * 1000 / u128::from(counts.total)).unwrap_or(1000)
    };
    Notice {
        title: errand.title.clone(),
        text,
        permille: permille.min(1000),
        waiting,
    }
}

/// Asks, once, to show notifications (Android 13 and later), at the moment a
/// transfer is started, which is when the reason is plain. Answers at once:
/// the run does not wait on the person, and goes ahead without if refused.
pub fn ask_to_notify(app: &AppHandle) {
    #[cfg(mobile)]
    {
        use tauri_plugin_photos::PhotosExt;
        let _ = app.photos().ask_notifications();
    }
    #[cfg(not(mobile))]
    let _ = app;
}

pub struct Keepalive {
    app: AppHandle,
    stop: Arc<Stop>,
    errand: Errand,
    last: Option<Instant>,
}

impl Keepalive {
    /// The run has begun: keep the screen on while the app is open, and the
    /// process alive while it is not.
    pub fn begin(app: &AppHandle, stop: Arc<Stop>, errand: Errand) -> Self {
        let keep = Self {
            app: app.clone(),
            stop,
            errand,
            last: None,
        };
        keep.phone_began();
        keep
    }

    /// The next transfer in the queue goes somewhere else.
    pub fn retitle(&mut self, errand: Errand) {
        self.errand = errand;
        self.last = None;
    }

    /// Updates the notification at most once a [`BEAT`], unless `now` (a
    /// change worth showing at once, like starting to wait). A Stop pressed
    /// there is handed to the run as Stop after this file.
    pub fn pulse(&mut self, counts: &Counts, waiting: bool, now: bool) {
        if !now && self.last.is_some_and(|at| at.elapsed() < BEAT) {
            return;
        }
        self.last = Some(Instant::now());
        if self.phone_progress(&notice(&self.errand, counts, waiting)) {
            self.stop.after_this_file();
        }
    }
}

#[cfg(mobile)]
impl Keepalive {
    fn phone_began(&self) {
        use tauri_plugin_photos::PhotosExt;
        let photos = self.app.photos();
        let _ = photos.keep_awake(true);
        if let Err(error) = photos.run_began(&notice(&self.errand, &Counts::default(), false)) {
            // The transfer runs regardless; it is only less sure to survive
            // the screen going off.
            tracing::warn!(%error, "could not keep the transfer going in the background");
        }
    }

    fn phone_progress(&self, notice: &Notice) -> bool {
        use tauri_plugin_photos::PhotosExt;
        self.app.photos().run_progress(notice).unwrap_or(false)
    }
}

#[cfg(not(mobile))]
impl Keepalive {
    #[allow(clippy::unused_self)]
    fn phone_began(&self) {
        let _ = &self.app;
    }

    #[allow(clippy::unused_self)]
    fn phone_progress(&self, _notice: &Notice) -> bool {
        false
    }
}

impl Drop for Keepalive {
    fn drop(&mut self) {
        #[cfg(mobile)]
        {
            use tauri_plugin_photos::PhotosExt;
            let photos = self.app.photos();
            let _ = photos.run_ended();
            let _ = photos.keep_awake(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(settled: usize, files: usize, bytes: u64, total: u64) -> Counts {
        Counts {
            files,
            settled,
            bytes,
            total,
            current: Some("DCIM/Camera/IMG_2041.jpg".into()),
        }
    }

    #[test]
    fn says_how_many_of_how_many_and_the_file_in_hand() {
        let n = notice(
            &Errand::transfer(false, "Area51"),
            &counts(34, 200, 1, 4),
            false,
        );
        assert_eq!(n.title, "Copying to Area51");
        assert_eq!(n.text, "34 of 200 · IMG_2041.jpg");
        assert_eq!(n.permille, 250);
    }

    #[test]
    fn names_the_far_side_as_the_window_does() {
        assert_eq!(
            Errand::transfer(false, "Docker:").title,
            "Copying to Docker"
        );
        assert_eq!(
            Errand::transfer(false, "Docker:Photos/2026").title,
            "Copying to Docker › Photos › 2026"
        );
        assert_eq!(
            Errand::transfer(false, "/storage/emulated/0/DCIM").title,
            "Copying to DCIM"
        );
    }

    #[test]
    fn a_move_says_moving() {
        assert_eq!(Errand::transfer(true, "Area51").title, "Moving to Area51");
    }

    #[test]
    fn waiting_names_what_it_waits_for() {
        let n = notice(
            &Errand::transfer(false, "Area51"),
            &counts(34, 200, 1, 4),
            true,
        );
        assert!(n.waiting);
        assert!(n.text.starts_with("Waiting for Area51 to answer"));
    }

    #[test]
    fn the_bar_never_overflows_or_runs_past_the_end() {
        let huge = counts(1, 2, u64::MAX, u64::MAX);
        assert_eq!(
            notice(&Errand::transfer(false, "x"), &huge, false).permille,
            1000
        );
        let none = counts(0, 0, 0, 0);
        assert_eq!(
            notice(&Errand::transfer(false, "x"), &none, false).text,
            "Getting ready"
        );
    }
}
