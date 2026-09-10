use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tungstate_backend::local::LocalBackend;
use tungstate_backend::{Backend, WriteFinish};
use tungstate_journal::{
    ConflictAction, Journal, Link, LinkId, NewLink, Order, SourcePolicy, VerifyLevel,
};

use super::*;

/// A whole drain, wired up over two temp directories.
struct Rig {
    source_dir: tempfile::TempDir,
    dest_dir: tempfile::TempDir,
    _journal_dir: tempfile::TempDir,
    source: LocalBackend,
    destination: LocalBackend,
    journal: Journal,
    link: Link,
}

impl Rig {
    fn new(policy: SourcePolicy) -> Self {
        Self::with(policy, VerifyLevel::Hash, Order::LargestFirst)
    }

    fn with(policy: SourcePolicy, verify: VerifyLevel, order: Order) -> Self {
        let source_dir = tempfile::tempdir().unwrap();
        let dest_dir = tempfile::tempdir().unwrap();
        let journal_dir = tempfile::tempdir().unwrap();

        let journal = Journal::open(&journal_dir.path().join("journal.db")).unwrap();
        let id = journal
            .create_link(&NewLink {
                name: "test-link".to_string(),
                source_root: source_dir.path().to_path_buf(),
                destination_root: dest_dir.path().to_path_buf(),
                source_policy: policy,
                verify,
                order,
                on_conflict: ConflictAction::Quarantine,
                // Zero, or every freshly written test file would be held back.
                cooldown: Duration::ZERO,
                saved: true,
            })
            .unwrap();
        let link = journal.link_by_name("test-link").unwrap();
        assert_eq!(link.id, id);

        Self {
            source: LocalBackend::new(source_dir.path().to_path_buf()),
            destination: LocalBackend::new(dest_dir.path().to_path_buf()),
            source_dir,
            dest_dir,
            _journal_dir: journal_dir,
            journal,
            link,
        }
    }

    fn write_source(&self, path: &str, contents: &[u8]) {
        let full = self.source_dir.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }

    fn write_dest(&self, path: &str, contents: &[u8]) {
        let full = self.dest_dir.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }

    fn dest(&self, path: &str) -> PathBuf {
        self.dest_dir.path().join(path)
    }

    fn src(&self, path: &str) -> PathBuf {
        self.source_dir.path().join(path)
    }

    fn run(&self) -> Result<Summary> {
        self.run_with(&mut FixedResolver(ConflictAction::Quarantine))
    }

    fn run_with(&self, resolver: &mut dyn ConflictResolver) -> Result<Summary> {
        let mut progress = SilentProgress;
        Transfer::new(
            &self.link,
            &self.source,
            &self.destination,
            &self.journal,
            resolver,
            &mut progress,
        )
        .run()
    }

    fn run_over(&self, destination: &dyn Backend) -> Result<Summary> {
        let mut progress = SilentProgress;
        let mut resolver = FixedResolver(ConflictAction::Quarantine);
        Transfer::new(
            &self.link,
            &self.source,
            destination,
            &self.journal,
            &mut resolver,
            &mut progress,
        )
        .run()
    }
}

#[test]
fn moves_files_and_reclaims_the_source() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"a video");
    rig.write_source("2024/trip.mp4", b"another video");

    let summary = rig.run().unwrap();

    assert_eq!(summary.transferred, 2);
    assert_eq!(summary.bytes, 20);
    assert_eq!(std::fs::read(rig.dest("holiday.mp4")).unwrap(), b"a video");
    assert_eq!(
        std::fs::read(rig.dest("2024/trip.mp4")).unwrap(),
        b"another video"
    );
    assert!(!rig.src("holiday.mp4").exists(), "source must be reclaimed");
    assert!(!rig.src("2024/trip.mp4").exists());
}

#[test]
fn copy_mode_leaves_every_original_alone() {
    let rig = Rig::new(SourcePolicy::Keep);
    rig.write_source("holiday.mp4", b"a video");

    rig.run().unwrap();

    assert!(rig.src("holiday.mp4").exists(), "--copy must not remove");
    assert!(rig.dest("holiday.mp4").exists());
}

#[test]
fn no_temporary_files_survive_a_successful_run() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"a video");
    rig.run().unwrap();

    let leftovers: Vec<_> = std::fs::read_dir(rig.dest_dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("tungstate"))
        .collect();
    assert!(leftovers.is_empty(), "left partials behind: {leftovers:?}");
}

/// Destination that writes rubbish, to prove verification actually gates deletion.
#[derive(Debug)]
struct CorruptingBackend(LocalBackend);

impl Backend for CorruptingBackend {
    fn capabilities(&self) -> tungstate_backend::Capabilities {
        self.0.capabilities()
    }
    fn root_token(&self) -> tungstate_backend::Result<tungstate_backend::RootToken> {
        self.0.root_token()
    }
    fn stat(&self, path: &Path) -> tungstate_backend::Result<tungstate_backend::Meta> {
        self.0.stat(path)
    }
    fn read_dir(&self, path: &Path) -> tungstate_backend::Result<Vec<tungstate_backend::Entry>> {
        self.0.read_dir(path)
    }
    fn open_read(&self, path: &Path) -> tungstate_backend::Result<Box<dyn std::io::Read + Send>> {
        self.0.open_read(path)
    }
    fn create_write(&self, path: &Path) -> tungstate_backend::Result<Box<dyn WriteFinish>> {
        Ok(Box::new(Corrupt(self.0.create_write(path)?)))
    }
    fn rename(&self, from: &Path, to: &Path) -> tungstate_backend::Result<()> {
        self.0.rename(from, to)
    }
    fn remove_file(&self, path: &Path) -> tungstate_backend::Result<()> {
        self.0.remove_file(path)
    }
    fn remove_dir(&self, path: &Path) -> tungstate_backend::Result<()> {
        self.0.remove_dir(path)
    }
    fn create_dir_all(&self, path: &Path) -> tungstate_backend::Result<()> {
        self.0.create_dir_all(path)
    }
}

struct Corrupt(Box<dyn WriteFinish>);

impl Write for Corrupt {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Same length, different bytes: only a readback catches this.
        let flipped: Vec<u8> = buf.iter().map(|b| b ^ 0xFF).collect();
        self.0.write_all(&flipped)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl WriteFinish for Corrupt {
    fn finish(self: Box<Self>) -> tungstate_backend::Result<()> {
        self.0.finish()
    }
}

#[test]
fn a_source_survives_when_verification_fails() {
    // The single most important guarantee in the product: nothing is deleted
    // until the copy is proven good.
    let rig = Rig::with(
        SourcePolicy::Delete,
        VerifyLevel::Readback,
        Order::LargestFirst,
    );
    rig.write_source("holiday.mp4", b"irreplaceable");

    let corrupting = CorruptingBackend(LocalBackend::new(rig.dest_dir.path().to_path_buf()));
    let summary = rig.run_over(&corrupting).unwrap();

    assert_eq!(
        summary.failed, 1,
        "the failure must be reported, not swallowed"
    );
    assert_eq!(summary.transferred, 0);
    assert!(
        rig.src("holiday.mp4").exists(),
        "source deleted despite failed verification"
    );
    assert!(
        !rig.dest("holiday.mp4").exists(),
        "a corrupt file must not be left under the real name"
    );
}

#[test]
fn a_failed_transfer_leaves_no_partial_file() {
    let rig = Rig::with(
        SourcePolicy::Delete,
        VerifyLevel::Readback,
        Order::LargestFirst,
    );
    rig.write_source("holiday.mp4", b"irreplaceable");

    let corrupting = CorruptingBackend(LocalBackend::new(rig.dest_dir.path().to_path_buf()));
    let _ = rig.run_over(&corrupting);

    let leftovers: Vec<_> = std::fs::read_dir(rig.dest_dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(leftovers.is_empty(), "partial left behind: {leftovers:?}");
}

#[test]
fn an_interrupted_run_resumes_without_duplicating_or_losing() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("done.mp4", b"already transferred");
    rig.write_source("pending.mp4", b"not yet");

    // First run completes normally.
    rig.run().unwrap();
    assert!(!rig.src("done.mp4").exists());

    // Simulate a crash mid-file: an intended op with a partial on disk.
    rig.write_source("interrupted.mp4", b"was in flight");
    let op = rig
        .journal
        .begin(&tungstate_journal::NewOp {
            kind: tungstate_journal::OpKind::Move,
            source: Some(tungstate_journal::Location::new(
                rig.link.source_root.clone(),
                "interrupted.mp4",
            )),
            destination: Some(tungstate_journal::Location::new(
                rig.link.destination_root.clone(),
                "interrupted.mp4",
            )),
            size: Some(13),
            link: Some(rig.link.name.clone()),
            link_id: Some(rig.link.id),
        })
        .unwrap();
    let partial = tungstate_journal::temp_name(Path::new("interrupted.mp4"), op);
    rig.write_dest(&partial.to_string_lossy(), b"half a fi");

    let summary = rig.run().unwrap();

    assert_eq!(summary.recovered, 1, "interrupted work must be recognised");
    assert_eq!(
        std::fs::read(rig.dest("interrupted.mp4")).unwrap(),
        b"was in flight",
        "the interrupted file must land complete"
    );
    assert!(
        !rig.dest_dir.path().join(&partial).exists(),
        "partial not cleaned up"
    );
    assert!(!rig.src("interrupted.mp4").exists());
    assert!(rig.journal.incomplete().unwrap().is_empty());
}

#[test]
fn an_identical_file_already_there_is_not_copied_again() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"same bytes");
    rig.write_dest("holiday.mp4", b"same bytes");

    let summary = rig.run().unwrap();

    assert_eq!(summary.already_present, 1);
    assert_eq!(summary.transferred, 0);
    assert!(
        !rig.src("holiday.mp4").exists(),
        "content is safely there, so the source should still be reclaimed"
    );
}

#[test]
fn a_conflicting_file_can_be_renamed_alongside() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"mine");
    rig.write_dest("holiday.mp4", b"theirs, different");

    let summary = rig
        .run_with(&mut FixedResolver(ConflictAction::Rename))
        .unwrap();

    assert_eq!(summary.transferred, 1);
    assert_eq!(
        std::fs::read(rig.dest("holiday.mp4")).unwrap(),
        b"theirs, different"
    );
    assert_eq!(std::fs::read(rig.dest("holiday-2.mp4")).unwrap(), b"mine");
}

#[test]
fn a_conflicting_file_can_be_skipped_leaving_the_source() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"mine");
    rig.write_dest("holiday.mp4", b"theirs, different");

    let summary = rig
        .run_with(&mut FixedResolver(ConflictAction::Skip))
        .unwrap();

    assert_eq!(summary.skipped, 1);
    assert!(rig.src("holiday.mp4").exists(), "skip must not reclaim");
    assert_eq!(
        std::fs::read(rig.dest("holiday.mp4")).unwrap(),
        b"theirs, different"
    );
}

#[test]
fn an_unattended_conflict_quarantines_and_still_reclaims() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"mine");
    rig.write_dest("holiday.mp4", b"theirs, different");

    let summary = rig
        .run_with(&mut FixedResolver(ConflictAction::Quarantine))
        .unwrap();

    assert_eq!(summary.quarantined, 1);
    assert_eq!(
        std::fs::read(rig.dest(".tungstate-quarantine/holiday.mp4")).unwrap(),
        b"mine",
        "the incoming file must be preserved, not discarded"
    );
    assert_eq!(
        std::fs::read(rig.dest("holiday.mp4")).unwrap(),
        b"theirs, different"
    );
    assert!(
        !rig.src("holiday.mp4").exists(),
        "space should still be reclaimed"
    );
}

#[test]
fn replace_quarantines_the_existing_file_rather_than_destroying_it() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"mine");
    rig.write_dest("holiday.mp4", b"theirs, different");

    rig.run_with(&mut FixedResolver(ConflictAction::Replace))
        .unwrap();

    assert_eq!(std::fs::read(rig.dest("holiday.mp4")).unwrap(), b"mine");
    assert_eq!(
        std::fs::read(rig.dest(".tungstate-quarantine/holiday.mp4")).unwrap(),
        b"theirs, different",
        "replace must not destroy what it replaced"
    );
}

#[test]
fn a_recently_written_file_is_left_for_the_next_run() {
    let mut rig = Rig::new(SourcePolicy::Delete);
    rig.link.cooldown = Duration::from_secs(3600);
    rig.write_source("still-downloading.mp4", b"partial");

    let summary = rig.run().unwrap();

    assert_eq!(summary.skipped, 1);
    assert_eq!(summary.transferred, 0);
    assert!(rig.src("still-downloading.mp4").exists());
}

#[test]
fn largest_first_reclaims_the_most_space_soonest() {
    let rig = Rig::with(SourcePolicy::Delete, VerifyLevel::Hash, Order::LargestFirst);
    rig.write_source("small.mp4", b"x");
    rig.write_source("large.mp4", &vec![b'x'; 5000]);
    rig.write_source("medium.mp4", &vec![b'x'; 500]);

    let mut seen = RecordingProgress::default();
    let mut resolver = FixedResolver(ConflictAction::Quarantine);
    Transfer::new(
        &rig.link,
        &rig.source,
        &rig.destination,
        &rig.journal,
        &mut resolver,
        &mut seen,
    )
    .run()
    .unwrap();

    assert_eq!(
        seen.order,
        vec![
            PathBuf::from("large.mp4"),
            PathBuf::from("medium.mp4"),
            PathBuf::from("small.mp4")
        ]
    );
}

#[derive(Default)]
struct RecordingProgress {
    order: Vec<PathBuf>,
}

impl Progress for RecordingProgress {
    fn starting(&mut self, path: &Path, _size: u64) {
        self.order.push(path.to_path_buf());
    }
    fn finished(&mut self, _path: &Path, _outcome: FileOutcome) {}
}

#[test]
fn emptied_source_directories_are_pruned() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("2024/trip/a.mp4", b"one");

    let summary = rig.run().unwrap();

    assert!(summary.pruned >= 1);
    assert!(
        !rig.src("2024/trip").exists(),
        "emptied directory should go"
    );
}

#[test]
fn a_directory_the_user_still_uses_is_not_pruned() {
    let rig = Rig::new(SourcePolicy::Keep);
    rig.write_source("2024/keep.txt", b"mine");

    rig.run().unwrap();

    assert!(rig.src("2024").exists(), "--copy must prune nothing");
}

#[test]
fn apply_to_all_stops_asking() {
    let answers = "s!\n";
    let mut output = Vec::new();
    let mut resolver =
        InteractiveResolver::new(answers.as_bytes(), &mut output, ConflictAction::Quarantine);

    let conflict = Conflict {
        path: PathBuf::from("a.mp4"),
        incoming_size: 1,
        existing_size: 2,
    };
    assert_eq!(resolver.resolve(&conflict), ConflictAction::Skip);
    // Input is exhausted; without the sticky answer this would hit the fallback.
    assert_eq!(resolver.resolve(&conflict), ConflictAction::Skip);
    assert_eq!(resolver.resolve(&conflict), ConflictAction::Skip);
}

#[test]
fn a_single_answer_applies_only_once() {
    let answers = "s\n";
    let mut output = Vec::new();
    let mut resolver =
        InteractiveResolver::new(answers.as_bytes(), &mut output, ConflictAction::Quarantine);

    let conflict = Conflict {
        path: PathBuf::from("a.mp4"),
        incoming_size: 1,
        existing_size: 2,
    };
    assert_eq!(resolver.resolve(&conflict), ConflictAction::Skip);
    // Input exhausted: nobody is there, so fall back rather than reuse the answer.
    assert_eq!(resolver.resolve(&conflict), ConflictAction::Quarantine);
}

#[test]
fn no_answer_at_all_falls_back_without_hanging() {
    let mut output = Vec::new();
    let mut resolver = InteractiveResolver::new(&b""[..], &mut output, ConflictAction::Quarantine);

    assert_eq!(
        resolver.resolve(&Conflict {
            path: PathBuf::from("a.mp4"),
            incoming_size: 1,
            existing_size: 2,
        }),
        ConflictAction::Quarantine
    );
}

#[test]
fn every_transfer_is_recorded_in_the_journal() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"a video");
    rig.run().unwrap();

    let history = rig.journal.history(Path::new("holiday.mp4")).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, tungstate_journal::OpStatus::Committed);
    assert!(history[0].hash.is_some(), "the digest must be recorded");
    assert_eq!(history[0].link.as_deref(), Some("test-link"));
}

#[test]
fn a_symlink_in_the_source_is_never_followed() {
    let rig = Rig::new(SourcePolicy::Keep);
    rig.write_source("real.mp4", b"content");

    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/hosts", rig.src("link.mp4")).unwrap();

    let summary = rig.run().unwrap();

    assert_eq!(summary.transferred, 1, "only the real file should move");
    assert!(!rig.dest("link.mp4").exists());
}

#[test]
fn an_empty_source_is_not_an_error() {
    let rig = Rig::new(SourcePolicy::Delete);
    assert_eq!(rig.run().unwrap(), Summary::default());
}

#[allow(dead_code)]
fn unused(_: LinkId) {}

#[test]
fn one_bad_file_does_not_abandon_the_rest_of_the_drain() {
    // A GUI watching four thousand files must not appear to die because one of
    // them is unreadable.
    let rig = Rig::with(
        SourcePolicy::Delete,
        VerifyLevel::Readback,
        Order::Discovered,
    );
    rig.write_source("a.mp4", b"first");
    rig.write_source("b.mp4", b"second");
    rig.write_source("c.mp4", b"third");

    // Corrupts every write, so all three fail verification.
    let corrupting = CorruptingBackend(LocalBackend::new(rig.dest_dir.path().to_path_buf()));
    let summary = rig.run_over(&corrupting).unwrap();

    assert_eq!(summary.failed, 3, "every file should have been attempted");
    assert_eq!(summary.failures.len(), 3);
    for name in ["a.mp4", "b.mp4", "c.mp4"] {
        assert!(
            rig.src(name).exists(),
            "{name} must survive a failed transfer"
        );
    }
}

#[test]
fn a_failure_carries_a_reason_worth_reading() {
    let rig = Rig::with(
        SourcePolicy::Delete,
        VerifyLevel::Readback,
        Order::Discovered,
    );
    rig.write_source("a.mp4", b"first");

    let corrupting = CorruptingBackend(LocalBackend::new(rig.dest_dir.path().to_path_buf()));
    let summary = rig.run_over(&corrupting).unwrap();

    assert_eq!(summary.failures[0].path, Path::new("a.mp4"));
    assert!(
        summary.failures[0].reason.contains("verification"),
        "reason should name what went wrong, got: {}",
        summary.failures[0].reason
    );
}

#[test]
fn a_cancelled_run_stops_cleanly_and_keeps_what_it_finished() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let rig = Rig::with(SourcePolicy::Delete, VerifyLevel::Hash, Order::Discovered);
    rig.write_source("a.mp4", b"first");
    rig.write_source("b.mp4", b"second");
    rig.write_source("c.mp4", b"third");

    let flag = Arc::new(AtomicBool::new(false));
    let mut stopper = StopAfterFirst {
        flag: Arc::clone(&flag),
    };
    let mut resolver = FixedResolver(ConflictAction::Quarantine);

    let summary = Transfer::new(
        &rig.link,
        &rig.source,
        &rig.destination,
        &rig.journal,
        &mut resolver,
        &mut stopper,
    )
    .cancellable(Arc::clone(&flag))
    .run()
    .unwrap();

    assert!(summary.cancelled);
    assert_eq!(summary.transferred, 1, "the in-flight file still completes");
    // Whatever it finished is committed; the rest are untouched, not half-done.
    assert_eq!(summary.transferred + summary.skipped, 1);
    let remaining = std::fs::read_dir(rig.source_dir.path()).unwrap().count();
    assert_eq!(remaining, 2, "unstarted files must be left alone");
    let _ = flag.load(Ordering::Relaxed);
}

struct StopAfterFirst {
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Progress for StopAfterFirst {
    fn starting(&mut self, _path: &Path, _size: u64) {}
    fn finished(&mut self, _path: &Path, _outcome: FileOutcome) {
        self.flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[test]
fn a_selection_moves_only_what_was_chosen() {
    // What the browser's Move does: the user picked these, not the folder.
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("wanted.mp4", b"chosen");
    rig.write_source("ignored.mp4", b"not chosen");
    rig.write_source("folder/inside.mp4", b"under a chosen folder");

    let mut resolver = FixedResolver(ConflictAction::Quarantine);
    let mut progress = SilentProgress;
    let summary = Transfer::new(
        &rig.link,
        &rig.source,
        &rig.destination,
        &rig.journal,
        &mut resolver,
        &mut progress,
    )
    .run_selection(&[PathBuf::from("wanted.mp4"), PathBuf::from("folder")])
    .unwrap();

    assert_eq!(summary.transferred, 2, "the file and the folder's contents");
    assert!(rig.dest("wanted.mp4").exists());
    assert!(rig.dest("folder/inside.mp4").exists());
    assert!(
        rig.src("ignored.mp4").exists(),
        "a file that was not chosen must be untouched"
    );
    assert!(!rig.dest("ignored.mp4").exists());
}

#[test]
fn an_empty_selection_does_nothing() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("a.mp4", b"x");

    let mut resolver = FixedResolver(ConflictAction::Quarantine);
    let mut progress = SilentProgress;
    let summary = Transfer::new(
        &rig.link,
        &rig.source,
        &rig.destination,
        &rig.journal,
        &mut resolver,
        &mut progress,
    )
    .run_selection(&[])
    .unwrap();

    assert_eq!(summary.transferred, 0);
    assert!(rig.src("a.mp4").exists());
}

/// A destination that reports different storage after the first file, which is
/// what an unmounting NAS looks like from here.
#[derive(Debug)]
struct VanishingBackend {
    inner: LocalBackend,
    calls: std::sync::atomic::AtomicU64,
}

impl Backend for VanishingBackend {
    fn capabilities(&self) -> tungstate_backend::Capabilities {
        self.inner.capabilities()
    }
    fn root_token(&self) -> tungstate_backend::Result<tungstate_backend::RootToken> {
        let n = self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // First two calls are the anchor and the first file's check.
        if n < 2 {
            self.inner.root_token()
        } else {
            Ok(tungstate_backend::RootToken { device: Some(999) })
        }
    }
    fn stat(&self, path: &Path) -> tungstate_backend::Result<tungstate_backend::Meta> {
        self.inner.stat(path)
    }
    fn read_dir(&self, path: &Path) -> tungstate_backend::Result<Vec<tungstate_backend::Entry>> {
        self.inner.read_dir(path)
    }
    fn open_read(&self, path: &Path) -> tungstate_backend::Result<Box<dyn std::io::Read + Send>> {
        self.inner.open_read(path)
    }
    fn create_write(&self, path: &Path) -> tungstate_backend::Result<Box<dyn WriteFinish>> {
        self.inner.create_write(path)
    }
    fn rename(&self, from: &Path, to: &Path) -> tungstate_backend::Result<()> {
        self.inner.rename(from, to)
    }
    fn remove_file(&self, path: &Path) -> tungstate_backend::Result<()> {
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &Path) -> tungstate_backend::Result<()> {
        self.inner.remove_dir(path)
    }
    fn create_dir_all(&self, path: &Path) -> tungstate_backend::Result<()> {
        self.inner.create_dir_all(path)
    }
}

#[test]
fn a_destination_that_changes_underneath_us_stops_the_drain() {
    // The catastrophic case: a NAS unmounts, its mount point becomes an empty
    // folder on the boot disk, and the drain fills the disk it was emptying
    // while deleting the originals. It must stop instead.
    let rig = Rig::with(SourcePolicy::Delete, VerifyLevel::Hash, Order::Discovered);
    for name in ["a.mp4", "b.mp4", "c.mp4", "d.mp4"] {
        rig.write_source(name, b"irreplaceable footage");
    }

    let vanishing = VanishingBackend {
        inner: LocalBackend::new(rig.dest_dir.path().to_path_buf()),
        calls: std::sync::atomic::AtomicU64::new(0),
    };
    let summary = rig.run_over(&vanishing).unwrap();

    assert!(summary.destination_lost, "the change must be noticed");
    assert!(
        summary.transferred <= 1,
        "at most the file already in flight, got {}",
        summary.transferred
    );

    let left = std::fs::read_dir(rig.source_dir.path()).unwrap().count();
    assert!(
        left >= 3,
        "originals must survive when the destination is not what it was, {left} left"
    );
}

#[test]
fn a_missing_destination_root_is_never_recreated() {
    // create_dir_all would otherwise rebuild a vanished mount point and write
    // into it, which is exactly how the files end up on the wrong disk.
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("unmounted");
    let backend = LocalBackend::new(gone.clone());

    let result = backend.create_dir_all(Path::new("2024"));

    assert!(matches!(
        result,
        Err(tungstate_backend::BackendError::RootUnreachable(_))
    ));
    assert!(!gone.exists(), "the root must not have been conjured up");
}

#[test]
fn a_preview_changes_absolutely_nothing() {
    // The whole point: you can look before you leap, and looking is free.
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("a.mp4", b"aaaa");
    rig.write_source("sub/b.mp4", b"bbbb");

    let before: Vec<_> = std::fs::read_dir(rig.source_dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();

    let preview = preview(&rig.link, &rig.source, &rig.destination, None).unwrap();

    assert_eq!(preview.fresh, 2);
    assert_eq!(preview.bytes, 8);
    assert!(preview.removes_originals);

    let after: Vec<_> = std::fs::read_dir(rig.source_dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(before, after, "the source must be untouched");
    assert_eq!(
        std::fs::read_dir(rig.dest_dir.path()).unwrap().count(),
        0,
        "the destination must be untouched"
    );
}

#[test]
fn a_preview_distinguishes_a_clash_from_a_same_size_file() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("same.mp4", b"1234");
    rig.write_dest("same.mp4", b"5678");
    rig.write_source("clash.mp4", b"1234");
    rig.write_dest("clash.mp4", b"much longer contents");
    rig.write_source("new.mp4", b"1234");

    let preview = preview(&rig.link, &rig.source, &rig.destination, None).unwrap();

    assert_eq!(preview.fresh, 1);
    assert_eq!(
        preview.same_size, 1,
        "same size is reported, not guessed at"
    );
    assert_eq!(preview.clashes, 1);

    let by_name = |n: &str| {
        preview
            .items
            .iter()
            .find(|i| i.path == Path::new(n))
            .unwrap()
            .prospect
            .clone()
    };
    assert_eq!(by_name("new.mp4"), Prospect::Fresh);
    assert_eq!(by_name("same.mp4"), Prospect::SameSize { existing: 4 });
    assert_eq!(by_name("clash.mp4"), Prospect::Clash { existing: 20 });
}

#[test]
fn a_preview_reports_what_the_cooldown_would_hold_back() {
    let mut rig = Rig::new(SourcePolicy::Delete);
    rig.link.cooldown = Duration::from_secs(3600);
    rig.write_source("downloading.mp4", b"partial");

    let preview = preview(&rig.link, &rig.source, &rig.destination, None).unwrap();

    assert_eq!(preview.too_recent, 1);
    assert_eq!(
        preview.bytes, 0,
        "held-back files are not counted as moving"
    );
}

#[test]
fn a_preview_of_an_unreachable_destination_says_so() {
    // Worth knowing before agreeing to anything, not after.
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("a.mp4", b"aaaa");
    let gone = LocalBackend::new(rig.dest_dir.path().join("not-mounted"));

    assert!(preview(&rig.link, &rig.source, &gone, None).is_err());
}

#[test]
fn a_preview_can_be_limited_to_a_selection() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("wanted.mp4", b"aaaa");
    rig.write_source("ignored.mp4", b"bbbb");

    let preview = preview(
        &rig.link,
        &rig.source,
        &rig.destination,
        Some(&[PathBuf::from("wanted.mp4")]),
    )
    .unwrap();

    assert_eq!(preview.items.len(), 1);
    assert_eq!(preview.items[0].path, Path::new("wanted.mp4"));
}
