//! The awkward halves, separately.
//!
//! The deadline arithmetic and the echo set are pure, so they are tested with
//! no filesystem at all. What a look at a folder decides is tested with a real
//! one and no watcher, because `sweep` is the same code path an event takes
//! and is deterministic. Only the last test involves the platform.

use std::path::Path;
use std::time::{Duration, Instant};

use super::*;

/// A governed folder with the given mode, and no waiting.
///
/// `cooldown = "0s"` so a file written a moment ago is not held back: this is
/// about what the watcher decides, and the planner's own settling has its own
/// tests in slice 6.
fn governed(mode: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(dir.path().join(".tungstate")).expect("dir");
    std::fs::write(
        dir.path().join(POLICY_RELATIVE),
        format!(
            "[folder]\nname = \"demo\"\nmode = \"{mode}\"\n\n\
             [defaults]\ncooldown = \"0s\"\n\n\
             [[rule]]\nname = \"by kind\"\npath = \"{{ext}}\"\n\n\
             [rule.match]\next = [\"jpg\", \"pdf\"]\n"
        ),
    )
    .expect("write");
    dir
}

fn watched(dir: &tempfile::TempDir) -> Vec<Watched> {
    vec![Watched {
        name: "demo".to_string(),
        root: dir.path().to_path_buf(),
        networked: false,
    }]
}

fn heard(folders: &[Watched], journal: &Journal) -> Vec<Noticed> {
    let mut seen = Vec::new();
    sweep(folders, journal, &mut |noticed| seen.push(noticed.clone()));
    seen
}

// --- the deadline arithmetic --------------------------------------------

#[test]
fn a_second_event_pushes_the_deadline_out() {
    // A file still being written keeps arriving, and the folder keeps being
    // put off until it stops. That is the whole of settling at this level.
    let mut schedule = Schedule::default();
    let start = Instant::now();
    schedule.stirred("/folder", start, Duration::from_secs(30));
    schedule.stirred(
        "/folder",
        start + Duration::from_secs(20),
        Duration::from_secs(30),
    );

    assert!(
        schedule.ready(start + Duration::from_secs(31)).is_empty(),
        "the first deadline should have been replaced, not kept"
    );
    assert_eq!(schedule.ready(start + Duration::from_secs(51)), ["/folder"]);
}

#[test]
fn a_folder_whose_moment_has_come_is_handed_over_once() {
    let mut schedule = Schedule::default();
    let start = Instant::now();
    schedule.stirred("/one", start, Duration::from_secs(1));
    schedule.stirred("/two", start, Duration::from_secs(10));

    assert_eq!(schedule.ready(start + Duration::from_secs(2)), ["/one"]);
    assert!(schedule.ready(start + Duration::from_secs(2)).is_empty());
    assert!(!schedule.is_empty(), "the other one is still waiting");
}

#[test]
fn the_wait_is_however_long_the_soonest_thing_is() {
    let mut schedule = Schedule::default();
    let start = Instant::now();
    assert_eq!(schedule.quiet_for(start), None, "nothing to wait for");

    schedule.stirred("/slow", start, Duration::from_secs(60));
    schedule.stirred("/soon", start, Duration::from_secs(5));

    let wait = schedule.quiet_for(start).expect("something is waiting");
    assert!(wait <= Duration::from_secs(5), "waited {wait:?}");
}

// --- the echo set --------------------------------------------------------

#[test]
fn our_own_writes_are_not_news() {
    let mut echoes = Echoes::default();
    let now = Instant::now();
    echoes.expect(
        ["/folder/jpg/holiday.jpg".to_string()],
        now + Duration::from_secs(10),
    );

    assert!(echoes.ours(Path::new("/folder/jpg/holiday.jpg"), now));
    assert!(!echoes.ours(Path::new("/folder/jpg/somebody-elses.jpg"), now));
}

#[test]
fn an_expectation_ages_out_rather_than_growing_for_ever() {
    let mut echoes = Echoes::default();
    let now = Instant::now();
    echoes.expect(["/folder/a".to_string()], now + Duration::from_secs(10));

    assert!(!echoes.ours(Path::new("/folder/a"), now + Duration::from_secs(11)));
    echoes.forget_old(now + Duration::from_secs(11));
    assert!(echoes.is_empty(), "{} left", echoes.len());
}

// --- which folder an event belongs to ------------------------------------

#[test]
fn the_deepest_governed_root_wins() {
    // A folder governed inside another folder gets its own events, rather
    // than waking its parent and being tidied by somebody else's rules.
    let roots = vec!["/home/me".to_string(), "/home/me/Downloads".to_string()];

    assert_eq!(
        root_of(Path::new("/home/me/Downloads/a.jpg"), &roots),
        Some("/home/me/Downloads")
    );
    assert_eq!(
        root_of(Path::new("/home/me/Desktop/a.jpg"), &roots),
        Some("/home/me")
    );
    assert_eq!(root_of(Path::new("/etc/hosts"), &roots), None);
}

#[test]
fn events_about_tungstates_own_directories_are_not_news() {
    // Without this, writing a plan into `.tungstate/` wakes the folder that
    // plan is about, for ever.
    assert!(is_ours(Path::new("/f/.tungstate/policy.toml"), "/f"));
    assert!(is_ours(Path::new("/f/.tungstate-quarantine/a.jpg"), "/f"));
    assert!(!is_ours(Path::new("/f/jpg/a.jpg"), "/f"));
}

#[test]
fn the_files_a_survey_probes_with_are_not_news() {
    // Every look writes these to learn what the filesystem can do. Heeding
    // them woke the folder once a second, for ever, while it was watched.
    assert!(is_ours(
        Path::new("/f/.tungstate-probe-link-70515-0-dst"),
        "/f"
    ));
    assert!(!is_ours(Path::new("/f/tungstate-notes.txt"), "/f"));
}

#[test]
fn an_event_about_the_root_itself_is_not_news() {
    // Its listing changed, which the child's own event already said. Probing
    // inside the root changes the root, so heeding this is the same loop.
    let folder = Watched {
        name: "demo".to_string(),
        root: "/f".into(),
        networked: false,
    };
    let resolved = vec![("/f".to_string(), &folder)];
    let roots = vec!["/f".to_string()];
    let mut schedule = Schedule::default();
    let now = Instant::now();

    note(
        Path::new("/f"),
        &roots,
        &resolved,
        &Echoes::default(),
        now,
        &mut schedule,
    );
    assert!(schedule.is_empty(), "the root's own event stirred it");

    note(
        Path::new("/f/holiday.jpg"),
        &roots,
        &resolved,
        &Echoes::default(),
        now,
        &mut schedule,
    );
    assert!(!schedule.is_empty(), "a file arriving should stir it");
}

// --- when the platform admits it lost events -----------------------------

fn two_folders() -> [Watched; 2] {
    [
        Watched {
            name: "photos".to_string(),
            root: "/home/me/Photos".into(),
            networked: false,
        },
        Watched {
            name: "downloads".to_string(),
            root: "/home/me/Downloads".into(),
            networked: false,
        },
    ]
}

#[test]
fn a_rescan_stirs_the_folder_it_names() {
    // FSEvents coalesced more than it could report and said only "look again
    // under here". Waiting for the hourly sweep would be right and an hour late.
    let folders = two_folders();
    let resolved: Vec<(String, &Watched)> = folders
        .iter()
        .map(|folder| (folder.root.to_string_lossy().to_string(), folder))
        .collect();
    let mut schedule = Schedule::default();
    let now = Instant::now();

    stir_all(&["/home/me/Photos".into()], &resolved, now, &mut schedule);

    let later = now + Duration::from_secs(3600);
    assert_eq!(schedule.ready(later), ["/home/me/Photos"]);
}

#[test]
fn a_loss_above_every_folder_stirs_all_of_them() {
    // A path that contains both roots, or none at all, could have hidden a
    // change in either.
    let folders = two_folders();
    let resolved: Vec<(String, &Watched)> = folders
        .iter()
        .map(|folder| (folder.root.to_string_lossy().to_string(), folder))
        .collect();
    let now = Instant::now();
    let later = now + Duration::from_secs(3600);

    for paths in [vec!["/home/me".into()], Vec::new()] {
        let mut schedule = Schedule::default();
        stir_all(&paths, &resolved, now, &mut schedule);
        assert_eq!(
            schedule.ready(later),
            ["/home/me/Downloads", "/home/me/Photos"],
            "{paths:?}"
        );
    }
}

#[test]
fn a_loss_somewhere_else_stirs_nothing() {
    let folders = two_folders();
    let resolved: Vec<(String, &Watched)> = folders
        .iter()
        .map(|folder| (folder.root.to_string_lossy().to_string(), folder))
        .collect();
    let mut schedule = Schedule::default();

    stir_all(&["/etc".into()], &resolved, Instant::now(), &mut schedule);

    assert!(schedule.is_empty());
}

// --- what a look at a folder decides -------------------------------------

#[test]
fn an_enforce_folder_files_what_arrived() {
    let dir = governed("enforce");
    std::fs::write(dir.path().join("holiday.jpg"), b"a picture").expect("write");
    let journal = Journal::open_in_memory().expect("journal");

    let seen = heard(&watched(&dir), &journal);

    assert!(
        matches!(seen.as_slice(), [Noticed::Tidied { files: 1, .. }]),
        "{seen:?}"
    );
    assert!(dir.path().join("jpg/holiday.jpg").exists());
}

#[test]
fn an_observe_folder_is_told_about_and_never_touched() {
    let dir = governed("observe");
    std::fs::write(dir.path().join("holiday.jpg"), b"a picture").expect("write");
    let journal = Journal::open_in_memory().expect("journal");

    let seen = heard(&watched(&dir), &journal);

    assert!(
        matches!(seen.as_slice(), [Noticed::Waiting { files: 1, .. }]),
        "{seen:?}"
    );
    assert!(
        dir.path().join("holiday.jpg").exists(),
        "nothing moves unless the folder says enforce"
    );
}

#[test]
fn a_suggest_folder_is_told_about_and_never_touched() {
    let dir = governed("suggest");
    std::fs::write(dir.path().join("holiday.jpg"), b"a picture").expect("write");
    let journal = Journal::open_in_memory().expect("journal");

    let seen = heard(&watched(&dir), &journal);

    assert!(
        matches!(seen.as_slice(), [Noticed::Waiting { .. }]),
        "{seen:?}"
    );
    assert!(dir.path().join("holiday.jpg").exists());
}

#[test]
fn a_folder_with_nothing_to_do_says_so_and_stops() {
    let dir = governed("enforce");
    let journal = Journal::open_in_memory().expect("journal");

    let seen = heard(&watched(&dir), &journal);

    assert!(
        matches!(seen.as_slice(), [Noticed::Settled { .. }]),
        "{seen:?}"
    );
}

#[test]
fn rules_that_will_not_load_are_reported_rather_than_fatal() {
    let dir = governed("enforce");
    std::fs::write(
        dir.path().join(POLICY_RELATIVE),
        "this is not toml at all [[[",
    )
    .expect("write");
    let journal = Journal::open_in_memory().expect("journal");

    let seen = heard(&watched(&dir), &journal);

    assert!(
        matches!(seen.as_slice(), [Noticed::Trouble { .. }]),
        "{seen:?}"
    );
}

#[test]
fn one_broken_folder_does_not_stop_the_others() {
    let broken = governed("enforce");
    std::fs::write(broken.path().join(POLICY_RELATIVE), "not toml [[[").expect("write");
    let working = governed("enforce");
    std::fs::write(working.path().join("holiday.jpg"), b"a picture").expect("write");
    let journal = Journal::open_in_memory().expect("journal");
    let folders = vec![
        Watched {
            name: "broken".to_string(),
            root: broken.path().to_path_buf(),
            networked: false,
        },
        Watched {
            name: "working".to_string(),
            root: working.path().to_path_buf(),
            networked: false,
        },
    ];

    let seen = heard(&folders, &journal);

    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(working.path().join("jpg/holiday.jpg").exists());
}

// --- the platform --------------------------------------------------------

#[test]
fn a_file_dropped_into_a_watched_folder_is_filed_without_anybody_asking() {
    // The one test that involves the operating system. Generous timeouts,
    // because FSEvents and inotify are allowed to take their time and a test
    // that fails on a busy machine is worse than no test.
    //
    // Two things at once: a file already sitting there when watching starts,
    // which no event will ever mention, and a file that arrives afterwards.
    let dir = governed("enforce");
    std::fs::write(dir.path().join("already-here.jpg"), b"a picture").expect("write");
    let journal = Journal::open_in_memory().expect("journal");
    let folders = watched(&dir);
    let stop = Stop::new();

    let (sender, heard) = std::sync::mpsc::channel();
    let ender = stop.clone();
    let root = dir.path().to_path_buf();
    let looking = std::thread::spawn(move || {
        let journal = journal;
        watch(
            &folders,
            &journal,
            Duration::from_secs(3600),
            &ender,
            &mut |noticed| {
                let _ = sender.send(noticed.clone());
            },
        )
    });

    // Wait for the watcher to be listening before making the change it is
    // supposed to hear, or the event happens before anything is watching.
    let started = heard
        .recv_timeout(Duration::from_secs(10))
        .expect("it says when it has started");
    assert!(
        matches!(started, Noticed::Started { watching: 1, .. }),
        "{started:?}"
    );
    // The opening sweep files what was already there, before anything is
    // dropped. Taking this for the dropped file's filing is the mistake this
    // test first made: it stopped watching before the event could arrive.
    let opening = heard
        .recv_timeout(Duration::from_secs(10))
        .expect("the opening sweep says what it did");
    assert!(
        matches!(opening, Noticed::Tidied { files: 1, .. }),
        "{opening:?}"
    );
    assert!(root.join("jpg/already-here.jpg").exists());
    std::thread::sleep(Duration::from_millis(500));
    std::fs::write(root.join("holiday.jpg"), b"a picture").expect("write");

    let mut filed = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        match heard.recv_timeout(Duration::from_secs(5)) {
            Ok(Noticed::Tidied { files, .. }) => {
                filed = Some(files);
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    stop.ask();
    let _ = looking.join();

    assert_eq!(filed, Some(1), "the file should have been filed on its own");
    assert!(root.join("jpg/holiday.jpg").exists());
}
