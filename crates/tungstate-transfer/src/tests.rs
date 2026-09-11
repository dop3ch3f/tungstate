use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tungstate_backend::local::LocalBackend;
use tungstate_backend::{Backend, WriteFinish};
use tungstate_journal::{
    ConflictAction, Endpoint, Journal, Link, LinkId, NewLink, Order, SourcePolicy, VerifyLevel,
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
                source: Endpoint::local(source_dir.path()),
                destination: Endpoint::local(dest_dir.path()),
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

    /// The rig's own destination directory, reached through `OpenDAL` instead of
    /// through `std::fs`.
    ///
    /// A real connection row in the rig's own journal, so this exercises the
    /// factory and the schema as well as the adapter.
    fn opendal_destination(&self) -> Box<dyn Backend> {
        let id = self
            .journal
            .create_connection(&tungstate_journal::NewConnection {
                name: format!("dest-{}", self.link.id.0),
                scheme: tungstate_journal::Scheme::Fs,
                host: None,
                port: None,
                username: None,
                root: self.dest_dir.path().to_string_lossy().into_owned(),
                options: std::collections::BTreeMap::new(),
            })
            .unwrap();

        tungstate_backend_opendal::open(
            &Endpoint::remote(id, PathBuf::new()),
            &self.journal,
            &tungstate_secret::MemoryStore::new(),
        )
        .unwrap()
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
///
/// Wraps any backend, so the same proof runs over `LocalBackend` and over the
/// `OpenDAL` adapter without being written twice.
struct CorruptingBackend(Box<dyn Backend>);

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

    let corrupting = CorruptingBackend(Box::new(LocalBackend::new(
        rig.dest_dir.path().to_path_buf(),
    )));
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

    let corrupting = CorruptingBackend(Box::new(LocalBackend::new(
        rig.dest_dir.path().to_path_buf(),
    )));
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
            source: Some(tungstate_journal::Location::within(
                &rig.link.source,
                "interrupted.mp4",
            )),
            destination: Some(tungstate_journal::Location::within(
                &rig.link.destination,
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
    let corrupting = CorruptingBackend(Box::new(LocalBackend::new(
        rig.dest_dir.path().to_path_buf(),
    )));
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

    let corrupting = CorruptingBackend(Box::new(LocalBackend::new(
        rig.dest_dir.path().to_path_buf(),
    )));
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
    use std::sync::atomic::AtomicBool;

    // More files than workers, deliberately. With several in flight,
    // "stop after this file" means "stop after the files in flight" — the
    // count is no longer one, and the guarantee that matters is not a count:
    // nothing half-done, and everything unstarted left exactly as it was.
    let rig = Rig::with(SourcePolicy::Delete, VerifyLevel::Hash, Order::Discovered);
    for i in 0..24 {
        rig.write_source(&format!("f{i:02}.mp4"), b"contents");
    }

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

    assert!(
        summary.cancelled,
        "the run must report that it stopped early"
    );
    assert!(
        summary.transferred < 24,
        "cancelling must actually stop something, got {}",
        summary.transferred
    );

    // Every file is either moved or still exactly where it was. Nothing is
    // in between, which is the only promise cancellation makes.
    let left = std::fs::read_dir(rig.source_dir.path()).unwrap().count();
    assert_eq!(
        usize::try_from(summary.transferred).unwrap_or(usize::MAX) + left,
        24,
        "{} moved and {left} left does not account for 24",
        summary.transferred
    );

    let partials: Vec<_> = std::fs::read_dir(rig.dest_dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("tungstate"))
        .collect();
    assert!(partials.is_empty(), "left partials behind: {partials:?}");
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
///
/// Wraps any backend, so the same proof runs over `LocalBackend` and over the
/// `OpenDAL` adapter.
struct VanishingBackend {
    inner: Box<dyn Backend>,
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
        inner: Box::new(LocalBackend::new(rig.dest_dir.path().to_path_buf())),
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

// ---------------------------------------------------------------------------
// The same engine, with the destination reached through OpenDAL rather than
// through `std::fs`. Same rig, same assertions; only the backend changes.
//
// Slice 4b ships no protocol, so these run on all three platforms with no
// server and no network. When slice 4c adds FTP, the question "does the
// adapter behave like `LocalBackend`?" is already answered here.
// ---------------------------------------------------------------------------

#[test]
fn a_whole_drain_works_through_the_opendal_adapter() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"a video");
    rig.write_source("2024/trip.mp4", b"another video");

    let summary = rig.run_over(rig.opendal_destination().as_ref()).unwrap();

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
fn a_source_survives_a_failed_verification_through_the_adapter() {
    // The single most important guarantee in the product, asserted again over
    // the new seam. A remote that quietly mangles bytes must not cost a file.
    let rig = Rig::with(
        SourcePolicy::Delete,
        VerifyLevel::Readback,
        Order::LargestFirst,
    );
    rig.write_source("holiday.mp4", b"irreplaceable");

    let corrupting = CorruptingBackend(rig.opendal_destination());
    let summary = rig.run_over(&corrupting).unwrap();

    assert_eq!(summary.failed, 1);
    assert_eq!(summary.transferred, 0);
    assert!(
        rig.src("holiday.mp4").exists(),
        "source deleted despite failed verification"
    );
    assert!(!rig.dest("holiday.mp4").exists());

    let leftovers: Vec<_> = std::fs::read_dir(rig.dest_dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(leftovers.is_empty(), "partial left behind: {leftovers:?}");
}

#[test]
fn an_interrupted_run_resumes_through_the_adapter() {
    let rig = Rig::new(SourcePolicy::Delete);
    let destination = rig.opendal_destination();

    rig.write_source("done.mp4", b"already transferred");
    rig.run_over(destination.as_ref()).unwrap();
    assert!(!rig.src("done.mp4").exists());

    // Simulate a crash mid-file: an intended op with a partial on disk.
    rig.write_source("interrupted.mp4", b"was in flight");
    let op = rig
        .journal
        .begin(&tungstate_journal::NewOp {
            kind: tungstate_journal::OpKind::Move,
            source: Some(tungstate_journal::Location::within(
                &rig.link.source,
                "interrupted.mp4",
            )),
            destination: Some(tungstate_journal::Location::within(
                &rig.link.destination,
                "interrupted.mp4",
            )),
            size: Some(13),
            link: Some(rig.link.name.clone()),
            link_id: Some(rig.link.id),
        })
        .unwrap();
    let partial = tungstate_journal::temp_name(Path::new("interrupted.mp4"), op);
    rig.write_dest(&partial.to_string_lossy(), b"half a fi");

    let summary = rig.run_over(destination.as_ref()).unwrap();

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
fn an_identical_file_is_recognised_through_the_adapter() {
    // Hashing both ends is the only way to know, so this is the test that
    // proves reads through the adapter return the same bytes writes put there.
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"same bytes");
    rig.write_dest("holiday.mp4", b"same bytes");

    let summary = rig.run_over(rig.opendal_destination().as_ref()).unwrap();

    assert_eq!(summary.already_present, 1);
    assert_eq!(summary.transferred, 0);
    assert!(!rig.src("holiday.mp4").exists());
}

#[test]
fn a_conflict_is_quarantined_through_the_adapter() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"mine");
    rig.write_dest("holiday.mp4", b"theirs, different");

    let summary = rig.run_over(rig.opendal_destination().as_ref()).unwrap();

    assert_eq!(summary.quarantined, 1);
    assert_eq!(
        std::fs::read(rig.dest(".tungstate-quarantine/holiday.mp4")).unwrap(),
        b"mine"
    );
    assert_eq!(
        std::fs::read(rig.dest("holiday.mp4")).unwrap(),
        b"theirs, different"
    );
}

#[test]
fn a_multi_chunk_file_drains_intact_through_the_adapter() {
    // Larger than the engine's 1 MiB read, so the streaming loop and the
    // adapter's chunked reads and writes are both genuinely exercised.
    let rig = Rig::new(SourcePolicy::Delete);
    let payload: Vec<u8> = (0..2_500_000_u32).map(|n| (n % 251) as u8).collect();
    rig.write_source("big.bin", &payload);

    let summary = rig.run_over(rig.opendal_destination().as_ref()).unwrap();

    assert_eq!(summary.transferred, 1);
    assert_eq!(summary.bytes, payload.len() as u64);
    assert_eq!(std::fs::read(rig.dest("big.bin")).unwrap(), payload);
}

#[test]
fn a_remote_source_refuses_to_trash_rather_than_deleting_the_wrong_thing() {
    // `trash::delete` drives the local desktop's trash. Handed a path relative
    // to a remote it would either fail obscurely or, worse, find a local file
    // of that name. The engine refuses before it gets the chance.
    let rig = Rig::new(SourcePolicy::Trash);
    rig.write_source("holiday.mp4", b"irreplaceable");

    let journal = &rig.journal;
    let id = journal
        .create_connection(&tungstate_journal::NewConnection {
            name: "pretend-remote".to_string(),
            scheme: tungstate_journal::Scheme::Fs,
            host: None,
            port: None,
            username: None,
            root: rig.source_dir.path().to_string_lossy().into_owned(),
            options: std::collections::BTreeMap::new(),
        })
        .unwrap();

    let link = Link {
        source: Endpoint::remote(id, PathBuf::new()),
        ..rig.link.clone()
    };
    let mut resolver = FixedResolver(ConflictAction::Quarantine);
    let mut progress = SilentProgress;
    let summary = Transfer::new(
        &link,
        &rig.source,
        &rig.destination,
        journal,
        &mut resolver,
        &mut progress,
    )
    .run()
    .unwrap();

    assert_eq!(summary.failed, 1, "the refusal must be reported");
    assert!(
        rig.src("holiday.mp4").exists(),
        "the original must still be there"
    );
    assert!(
        summary.failures[0].reason.contains("no trash"),
        "the reason must name the problem, got `{}`",
        summary.failures[0].reason
    );
}

#[test]
fn a_connection_that_changes_underneath_us_stops_the_drain() {
    // The catastrophic case again, over the seam. It is worth repeating here
    // rather than trusting the local version: `OpendalBackend::root_token` had
    // to avoid `Operator::stat("/")`, which OpenDAL answers from thin air
    // without touching the store. Had it not, this test would pass four files
    // to a destination that was no longer there and delete four originals.
    let rig = Rig::with(SourcePolicy::Delete, VerifyLevel::Hash, Order::Discovered);
    for name in ["a.mp4", "b.mp4", "c.mp4", "d.mp4"] {
        rig.write_source(name, b"irreplaceable footage");
    }

    let vanishing = VanishingBackend {
        inner: rig.opendal_destination(),
        calls: std::sync::atomic::AtomicU64::new(0),
    };
    let summary = rig.run_over(&vanishing).unwrap();

    assert!(summary.destination_lost, "the change must be noticed");
    assert!(summary.transferred <= 1, "got {}", summary.transferred);

    let left = std::fs::read_dir(rig.source_dir.path()).unwrap().count();
    assert!(left >= 3, "originals must survive, {left} left");
}

// ---------------------------------------------------------------------------
// Backends that cannot rename. FTP is the reason these exist, but nothing here
// needs FTP: a fake that reports `atomic_rename: false` exercises every engine
// path that FTP will take, on all three platforms with no server.
// ---------------------------------------------------------------------------

/// Any backend, with rename removed.
///
/// Both halves matter. Reporting `atomic_rename: false` is what steers the
/// engine, and making `rename` actually fail is what proves the engine really
/// stopped calling it rather than merely reading the flag.
struct NoRenameBackend(Box<dyn Backend>);

impl NoRenameBackend {
    fn local(root: &Path) -> Self {
        Self(Box::new(LocalBackend::new(root.to_path_buf())))
    }
}

impl Backend for NoRenameBackend {
    fn capabilities(&self) -> tungstate_backend::Capabilities {
        tungstate_backend::Capabilities {
            atomic_rename: false,
            ..self.0.capabilities()
        }
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
        self.0.create_write(path)
    }
    fn rename(&self, from: &Path, _to: &Path) -> tungstate_backend::Result<()> {
        Err(tungstate_backend::BackendError::Remote {
            endpoint: "no-rename".to_string(),
            operation: "rename",
            source: format!("`{}` cannot be renamed here", from.display()).into(),
        })
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

#[test]
fn a_destination_that_cannot_rename_still_drains() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"a video");
    rig.write_source("2024/trip.mp4", b"another video");

    let summary = rig
        .run_over(&NoRenameBackend::local(rig.dest_dir.path()))
        .unwrap();

    assert_eq!(summary.transferred, 2);
    assert_eq!(std::fs::read(rig.dest("holiday.mp4")).unwrap(), b"a video");
    assert_eq!(
        std::fs::read(rig.dest("2024/trip.mp4")).unwrap(),
        b"another video"
    );
    assert!(!rig.src("holiday.mp4").exists());
}

#[test]
fn a_destination_that_cannot_rename_writes_no_part_file() {
    // The engine must not write to a temp name it can never move. Doing so
    // would leave a `.part` beside every file, forever.
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"a video");

    rig.run_over(&NoRenameBackend::local(rig.dest_dir.path()))
        .unwrap();

    let leftovers: Vec<_> = std::fs::read_dir(rig.dest_dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("tungstate"))
        .collect();
    assert!(leftovers.is_empty(), "left partials behind: {leftovers:?}");
}

#[test]
fn a_source_survives_a_failed_verification_without_rename() {
    // The guarantee the project exists for, on the path FTP will take.
    let rig = Rig::with(
        SourcePolicy::Delete,
        VerifyLevel::Readback,
        Order::LargestFirst,
    );
    rig.write_source("holiday.mp4", b"irreplaceable");

    let corrupting = CorruptingBackend(Box::new(NoRenameBackend::local(rig.dest_dir.path())));
    let summary = rig.run_over(&corrupting).unwrap();

    assert_eq!(summary.failed, 1);
    assert_eq!(summary.transferred, 0);
    assert!(
        rig.src("holiday.mp4").exists(),
        "source deleted despite failed verification"
    );
    assert!(
        !rig.dest("holiday.mp4").exists(),
        "the bad copy must not be left under the real name"
    );
}

#[test]
fn an_interrupted_run_resumes_without_rename() {
    // Without a temp name the partial can only be under the real name, so
    // recovery has to delete that instead. The source is untouched either way,
    // which is what makes deleting it safe.
    let rig = Rig::new(SourcePolicy::Delete);
    let destination = NoRenameBackend::local(rig.dest_dir.path());

    rig.write_source("interrupted.mp4", b"was in flight");
    let op = rig
        .journal
        .begin(&tungstate_journal::NewOp {
            kind: tungstate_journal::OpKind::Move,
            source: Some(tungstate_journal::Location::within(
                &rig.link.source,
                "interrupted.mp4",
            )),
            destination: Some(tungstate_journal::Location::within(
                &rig.link.destination,
                "interrupted.mp4",
            )),
            size: Some(13),
            link: Some(rig.link.name.clone()),
            link_id: Some(rig.link.id),
        })
        .unwrap();
    // A partial under the real name, plus the randomly-named orphan a backend
    // doing its own temp-and-rename would have left beside it.
    rig.write_dest("interrupted.mp4", b"half a fi");
    rig.write_dest("interrupted.mp4.k3xq9wpz", b"half a fi");
    let _ = op;

    let summary = rig.run_over(&destination).unwrap();

    assert_eq!(summary.recovered, 1);
    assert_eq!(
        std::fs::read(rig.dest("interrupted.mp4")).unwrap(),
        b"was in flight",
        "the interrupted file must land complete, not appended to"
    );
    assert!(
        !rig.dest("interrupted.mp4.k3xq9wpz").exists(),
        "the abandoned temporary file must be swept"
    );
    assert!(!rig.src("interrupted.mp4").exists());
    assert!(rig.journal.incomplete().unwrap().is_empty());
}

#[test]
fn the_sweep_only_removes_what_a_backend_actually_left() {
    // A pattern this narrow is the only thing standing between "tidy up after
    // a crash" and "delete one of the user's files".
    assert!(is_orphan_temp("a.mp4.k3xq9wpz", "a.mp4"));
    assert!(is_orphan_temp("a.mp4.00000000", "a.mp4"));

    for innocent in [
        "a.mp4",           // the file itself
        "a.mp4.txt",       // a real sibling, three characters
        "a.mp4.backup",    // six
        "a.mp4.k3xq9wpzz", // nine
        "a.mp4.k3xq9wp-",  // not alphanumeric
        "a.mp4k3xq9wpz",   // no separating dot
        "b.mp4.k3xq9wpz",  // a different file's temp
    ] {
        assert!(
            !is_orphan_temp(innocent, "a.mp4"),
            "`{innocent}` must survive the sweep"
        );
    }
}

#[test]
fn replace_is_refused_rather_than_destroying_the_file_it_promised_to_keep() {
    // Replace moves the existing file aside and then lands the incoming one.
    // Without rename the first half is impossible, and doing only the second
    // half is the exact opposite of what Replace undertakes.
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("holiday.mp4", b"mine");
    rig.write_dest("holiday.mp4", b"theirs, different");

    let destination = NoRenameBackend::local(rig.dest_dir.path());
    let mut resolver = FixedResolver(ConflictAction::Replace);
    let mut progress = SilentProgress;
    let summary = Transfer::new(
        &rig.link,
        &rig.source,
        &destination,
        &rig.journal,
        &mut resolver,
        &mut progress,
    )
    .run()
    .unwrap();

    assert_eq!(summary.failed, 1);
    assert_eq!(
        std::fs::read(rig.dest("holiday.mp4")).unwrap(),
        b"theirs, different",
        "the file Replace promised to keep must still be there"
    );
    assert!(rig.src("holiday.mp4").exists(), "and so must the source");
    assert!(
        summary.failures[0].reason.contains("cannot rename"),
        "the reason must say why, got `{}`",
        summary.failures[0].reason
    );
}

#[test]
fn quarantine_and_rename_still_work_without_rename_support() {
    // Neither needs to move an existing file, so both must keep working; only
    // Replace is affected.
    for (action, landing) in [
        (ConflictAction::Quarantine, ".tungstate-quarantine/a.mp4"),
        (ConflictAction::Rename, "a-2.mp4"),
    ] {
        let rig = Rig::new(SourcePolicy::Delete);
        rig.write_source("a.mp4", b"mine");
        rig.write_dest("a.mp4", b"theirs, different");

        let destination = NoRenameBackend::local(rig.dest_dir.path());
        let mut resolver = FixedResolver(action);
        let mut progress = SilentProgress;
        Transfer::new(
            &rig.link,
            &rig.source,
            &destination,
            &rig.journal,
            &mut resolver,
            &mut progress,
        )
        .run()
        .unwrap();

        assert_eq!(
            std::fs::read(rig.dest(landing)).unwrap(),
            b"mine",
            "{action:?} should have landed at {landing}"
        );
        assert_eq!(
            std::fs::read(rig.dest("a.mp4")).unwrap(),
            b"theirs, different"
        );
    }
}

// ---------------------------------------------------------------------------
// Interrupted work that nothing else can find.
//
// A one-off transfer from the browser makes an unsaved link. `links()` does
// not return those, and recovery is scoped per link, so before this existed a
// crash mid-file left an operation stuck at `intended` and a multi-gigabyte
// partial that nothing would ever look at again.
// ---------------------------------------------------------------------------

/// A `NewLink` pointing at the same two places the rig uses.
fn rig_link(rig: &Rig) -> NewLink {
    NewLink {
        name: "unused".to_string(),
        source: rig.link.source.clone(),
        destination: rig.link.destination.clone(),
        source_policy: rig.link.source_policy,
        verify: rig.link.verify,
        order: rig.link.order,
        on_conflict: rig.link.on_conflict,
        cooldown: rig.link.cooldown,
        saved: true,
    }
}

/// Leave the rig looking like a run that died mid-file.
fn strand(rig: &Rig, name: &str, partial: &[u8]) -> tungstate_journal::OpId {
    rig.write_source(name, b"the whole file, still here");
    let op = rig
        .journal
        .begin(&tungstate_journal::NewOp {
            kind: tungstate_journal::OpKind::Move,
            source: Some(tungstate_journal::Location::within(&rig.link.source, name)),
            destination: Some(tungstate_journal::Location::within(
                &rig.link.destination,
                name,
            )),
            size: Some(26),
            link: Some(rig.link.name.clone()),
            link_id: Some(rig.link.id),
        })
        .unwrap();
    let temp = tungstate_journal::temp_name(Path::new(name), op);
    rig.write_dest(&temp.to_string_lossy(), partial);
    op
}

#[test]
fn interrupted_work_is_found_even_on_a_link_the_saved_list_hides() {
    let rig = Rig::new(SourcePolicy::Delete);
    // Exactly what `start_transfer` creates for a browser move.
    let id = rig
        .journal
        .create_link(&NewLink {
            name: "browser-1789128635946-0".to_string(),
            saved: false,
            ..rig_link(&rig)
        })
        .unwrap();
    let op = rig
        .journal
        .begin(&tungstate_journal::NewOp {
            kind: tungstate_journal::OpKind::Move,
            source: Some(tungstate_journal::Location::within(
                &rig.link.source,
                "a.mp4",
            )),
            destination: Some(tungstate_journal::Location::within(
                &rig.link.destination,
                "a.mp4",
            )),
            size: Some(4_000_000_000),
            link: Some("browser-1789128635946-0".to_string()),
            link_id: Some(id),
        })
        .unwrap();

    assert!(
        !rig.journal.links().unwrap().iter().any(|l| l.id == id),
        "an unsaved link must stay out of the saved-pairs list"
    );

    let runs = rig.journal.interrupted().unwrap();
    let found = runs
        .iter()
        .find(|r| r.link.id == id)
        .expect("the unsaved link's unfinished work must still be findable");
    assert_eq!(found.ops.len(), 1);
    assert_eq!(found.ops[0].id, op);
    assert_eq!(found.bytes(), 4_000_000_000);
    assert_eq!(found.ops[0].link_id, Some(id));
}

#[test]
fn a_link_whose_work_all_finished_is_not_reported_as_interrupted() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("a.mp4", b"done");
    rig.run().unwrap();

    assert!(
        rig.journal.interrupted().unwrap().is_empty(),
        "a completed run must not look like unfinished work"
    );
}

#[test]
fn discarding_removes_the_partial_and_copies_nothing() {
    let rig = Rig::new(SourcePolicy::Delete);
    let op = strand(&rig, "huge.mp4", b"a few bytes of it");
    let partial = tungstate_journal::temp_name(Path::new("huge.mp4"), op);
    assert!(rig.dest_dir.path().join(&partial).exists());

    let removed = discard(&rig.link, &rig.destination, &rig.journal).unwrap();

    assert_eq!(removed.operations, 1);
    assert_eq!(removed.bytes, 26);
    assert!(
        !rig.dest_dir.path().join(&partial).exists(),
        "the partial must be gone"
    );
    assert!(
        !rig.dest("huge.mp4").exists(),
        "discarding must not copy the file across"
    );
    assert!(
        rig.src("huge.mp4").exists(),
        "and must never touch the original"
    );
    assert!(rig.journal.interrupted().unwrap().is_empty());
}

#[test]
fn discarding_without_rename_clears_the_real_name_and_sweeps() {
    // The FTP shape: no temp name, so the partial is under the real name, and
    // the backend may have left its own randomly-named orphan beside it.
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("huge.mp4", b"the whole file, still here");
    rig.journal
        .begin(&tungstate_journal::NewOp {
            kind: tungstate_journal::OpKind::Move,
            source: Some(tungstate_journal::Location::within(
                &rig.link.source,
                "huge.mp4",
            )),
            destination: Some(tungstate_journal::Location::within(
                &rig.link.destination,
                "huge.mp4",
            )),
            size: Some(26),
            link: Some(rig.link.name.clone()),
            link_id: Some(rig.link.id),
        })
        .unwrap();
    rig.write_dest("huge.mp4", b"half of it");
    rig.write_dest("huge.mp4.k3xq9wpz", b"half of it");
    rig.write_dest("huge.mp4.backup", b"not ours");

    let destination = NoRenameBackend::local(rig.dest_dir.path());
    discard(&rig.link, &destination, &rig.journal).unwrap();

    assert!(!rig.dest("huge.mp4").exists(), "the partial must be gone");
    assert!(
        !rig.dest("huge.mp4.k3xq9wpz").exists(),
        "and the backend's orphan with it"
    );
    assert_eq!(
        std::fs::read(rig.dest("huge.mp4.backup")).unwrap(),
        b"not ours",
        "a real sibling must survive"
    );
    assert!(rig.src("huge.mp4").exists());
}

#[test]
fn resuming_an_unsaved_link_finishes_what_it_started() {
    // The whole point: the file completes, the partial is gone, and the
    // original is reclaimed, all through a link the saved list never shows.
    let rig = Rig::new(SourcePolicy::Delete);
    let op = strand(&rig, "huge.mp4", b"a few bytes of it");
    let partial = tungstate_journal::temp_name(Path::new("huge.mp4"), op);

    let summary = rig.run().unwrap();

    assert_eq!(summary.recovered, 1, "its own old operation must be seen");
    assert_eq!(
        std::fs::read(rig.dest("huge.mp4")).unwrap(),
        b"the whole file, still here"
    );
    assert!(!rig.dest_dir.path().join(&partial).exists());
    assert!(!rig.src("huge.mp4").exists());
    assert!(rig.journal.interrupted().unwrap().is_empty());
}

#[test]
fn a_link_with_no_stored_selection_still_means_the_whole_source() {
    // What a saved folder-pair means, and what every link written before
    // migration v5 means. The counterpart below covers a stored selection.
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("chosen.mp4", b"a file");
    rig.write_source("never-picked-a.mp4", b"another file");
    rig.write_source("never-picked-b.mp4", b"a third file");

    // Exactly the state a killed browser transfer leaves: one interrupted op
    // for the picked file, and no record anywhere of what else was picked.
    rig.journal
        .begin(&tungstate_journal::NewOp {
            kind: tungstate_journal::OpKind::Move,
            source: Some(tungstate_journal::Location::within(
                &rig.link.source,
                "chosen.mp4",
            )),
            destination: Some(tungstate_journal::Location::within(
                &rig.link.destination,
                "chosen.mp4",
            )),
            size: Some(23),
            link: Some(rig.link.name.clone()),
            link_id: Some(rig.link.id),
        })
        .unwrap();

    // What `resume_interrupted` does: spawn_run with an empty selection.
    let summary = rig.run().unwrap();

    assert_eq!(summary.transferred, 3, "no selection means everything");
    assert!(!rig.src("never-picked-a.mp4").exists());
}

// ---------------------------------------------------------------------------
// A stored selection. The gap that made an interrupted browser transfer
// resume as "the whole folder" rather than as itself.
// ---------------------------------------------------------------------------

#[test]
fn a_stored_selection_is_the_only_thing_moved() {
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("picked-a.mp4", b"wanted");
    rig.write_source("picked-b.mp4", b"also wanted");
    rig.write_source("not-picked.mp4", b"never chosen");

    rig.journal
        .set_files(
            rig.link.id,
            &[PathBuf::from("picked-a.mp4"), PathBuf::from("picked-b.mp4")],
        )
        .unwrap();

    let summary = rig.run().unwrap();

    assert_eq!(summary.transferred, 2);
    assert!(rig.dest("picked-a.mp4").exists());
    assert!(rig.dest("picked-b.mp4").exists());
    assert!(
        !rig.dest("not-picked.mp4").exists(),
        "a file outside the selection must not be transferred"
    );
    assert!(
        rig.src("not-picked.mp4").exists(),
        "and certainly must not be deleted"
    );
}

#[test]
fn resuming_a_stored_selection_finishes_the_batch_and_nothing_else() {
    // The reported bug, end to end: tick several files, die partway, resume,
    // and get the rest of the batch rather than the rest of the folder.
    let rig = Rig::new(SourcePolicy::Delete);
    for name in ["a.mp4", "b.mp4", "c.mp4"] {
        rig.write_source(name, b"in the batch");
    }
    rig.write_source("bystander.mp4", b"never chosen");
    rig.journal
        .set_files(rig.link.id, &["a.mp4", "b.mp4", "c.mp4"].map(PathBuf::from))
        .unwrap();

    // One of the batch already went before the crash; another was in flight.
    rig.run_with(&mut FixedResolver(ConflictAction::Quarantine))
        .unwrap();
    assert!(!rig.src("a.mp4").exists());

    // Everything from the batch is now at the destination and gone from the
    // source, and the bystander was never touched.
    assert!(rig.dest("c.mp4").exists());
    assert!(
        rig.src("bystander.mp4").exists(),
        "a file outside the batch must survive the whole run"
    );
    assert!(!rig.dest("bystander.mp4").exists());

    // Running again is a no-op rather than a re-walk of the folder.
    let again = rig.run().unwrap();
    assert_eq!(again.transferred, 0);
    assert_eq!(
        again.already_present, 0,
        "the sources are gone, so nothing to do"
    );
    assert!(rig.src("bystander.mp4").exists());
}

/// Records everything the engine reports, so the order can be asserted.
#[derive(Default)]
struct Recorder {
    plan: Vec<Planned>,
    events: Vec<String>,
    advances: Vec<(String, u64, u64)>,
}

impl Progress for Recorder {
    fn planned(&mut self, files: &[Planned]) {
        self.plan = files.to_vec();
        self.events.push("planned".to_string());
    }
    fn starting(&mut self, path: &Path, _size: u64) {
        self.events.push(format!("starting {}", path.display()));
    }
    fn advanced(&mut self, path: &Path, done: u64, total: u64) {
        self.advances
            .push((path.display().to_string(), done, total));
    }
    fn finished(&mut self, path: &Path, _outcome: FileOutcome) {
        self.events.push(format!("finished {}", path.display()));
    }
}

#[test]
fn the_whole_plan_is_announced_once_before_the_first_file() {
    // Without this the window can only show what has already happened, which
    // is why an interrupted batch looked like a single file.
    let rig = Rig::with(SourcePolicy::Delete, VerifyLevel::Hash, Order::LargestFirst);
    rig.write_source("small.mp4", b"aa");
    rig.write_source("big.mp4", b"aaaaaaaaaa");

    let mut recorder = Recorder::default();
    let mut resolver = FixedResolver(ConflictAction::Quarantine);
    Transfer::new(
        &rig.link,
        &rig.source,
        &rig.destination,
        &rig.journal,
        &mut resolver,
        &mut recorder,
    )
    .run()
    .unwrap();

    assert_eq!(
        recorder.events.first().map(String::as_str),
        Some("planned"),
        "the plan must arrive before anything starts"
    );
    assert_eq!(
        recorder.events.iter().filter(|e| *e == "planned").count(),
        1
    );
    assert_eq!(
        recorder
            .plan
            .iter()
            .map(|p| p.path.display().to_string())
            .collect::<Vec<_>>(),
        vec!["big.mp4", "small.mp4"],
        "and in the order the run will take them"
    );
    assert_eq!(recorder.plan[0].size, 10);
}

#[test]
fn a_file_reports_its_progress_and_lands_exactly_on_its_total() {
    // Throttling must not cost the last event: a bar that stops at 97% reads
    // as a stall.
    let rig = Rig::new(SourcePolicy::Delete);
    let payload: Vec<u8> = (0..(5 * 1024 * 1024_u32))
        .map(|n| (n % 251) as u8)
        .collect();
    rig.write_source("big.bin", &payload);

    let mut recorder = Recorder::default();
    let mut resolver = FixedResolver(ConflictAction::Quarantine);
    Transfer::new(
        &rig.link,
        &rig.source,
        &rig.destination,
        &rig.journal,
        &mut resolver,
        &mut recorder,
    )
    .run()
    .unwrap();

    let last = recorder.advances.last().expect("progress was reported");
    assert_eq!(last.0, "big.bin");
    assert_eq!(
        last.1,
        payload.len() as u64,
        "the last report must be the whole file"
    );
    assert_eq!(last.1, last.2, "and done must equal total");

    // Five chunks pass through the loop; a quarter-second throttle means the
    // final report is normally the only one. What matters is that it is not
    // one per chunk.
    assert!(
        recorder.advances.len() <= 5,
        "expected throttling, got {} reports",
        recorder.advances.len()
    );
}

// ---------------------------------------------------------------------------
// Several files at once.
// ---------------------------------------------------------------------------

#[test]
fn a_parallel_run_moves_the_same_bytes_as_a_sequential_one() {
    // The only thing that would make concurrency unacceptable is a different
    // answer, so this asserts sameness rather than speed.
    let expected: Vec<(String, Vec<u8>)> = (0..40)
        .map(|i| {
            let name = format!("f{i:02}.bin");
            let body: Vec<u8> = (0..(600 + i * 37)).map(|n: u32| (n % 251) as u8).collect();
            (name, body)
        })
        .collect();

    let mut outcomes = Vec::new();
    for parallel in [1, 4] {
        let rig = Rig::with(
            SourcePolicy::Delete,
            VerifyLevel::Readback,
            Order::LargestFirst,
        );
        for (name, body) in &expected {
            rig.write_source(name, body);
        }

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
        .parallel(parallel)
        .run()
        .unwrap();

        for (name, body) in &expected {
            assert_eq!(
                &std::fs::read(rig.dest(name)).unwrap(),
                body,
                "{name} at {parallel}"
            );
            assert!(
                !rig.src(name).exists(),
                "{name} not reclaimed at {parallel}"
            );
        }
        outcomes.push((summary.transferred, summary.bytes, summary.failed));
    }

    assert_eq!(
        outcomes[0], outcomes[1],
        "sequential and parallel must agree"
    );
    assert_eq!(outcomes[0].0, 40);
}

/// Panics if asked two questions at once, which is what an unguarded
/// resolver would allow.
struct OneAtATime {
    inside: std::sync::Arc<AtomicBool>,
    asked: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl ConflictResolver for OneAtATime {
    fn resolve(&mut self, _conflict: &Conflict) -> ConflictAction {
        assert!(
            !self.inside.swap(true, Ordering::SeqCst),
            "two conflicts were asked at the same time"
        );
        // Long enough that an unguarded second caller would overlap.
        std::thread::sleep(std::time::Duration::from_millis(20));
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.inside.store(false, Ordering::SeqCst);
        ConflictAction::Rename
    }
}

#[test]
fn conflicts_are_asked_one_at_a_time_however_many_files_are_in_flight() {
    // Being asked two questions at once is worse than waiting for the first.
    let rig = Rig::new(SourcePolicy::Delete);
    for i in 0..8 {
        let name = format!("clash{i}.mp4");
        rig.write_source(&name, b"mine");
        rig.write_dest(&name, b"theirs, different");
    }

    let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut resolver = OneAtATime {
        inside: std::sync::Arc::new(AtomicBool::new(false)),
        asked: std::sync::Arc::clone(&asked),
    };
    let mut progress = SilentProgress;
    Transfer::new(
        &rig.link,
        &rig.source,
        &rig.destination,
        &rig.journal,
        &mut resolver,
        &mut progress,
    )
    .parallel(4)
    .run()
    .unwrap();

    assert_eq!(asked.load(Ordering::SeqCst), 8);
}

#[test]
fn a_renamed_file_never_lands_on_a_name_another_file_is_about_to_use() {
    // The reachable race, and it is concurrency-only. `holiday.mp4` clashes,
    // so it is renamed to `holiday-2.mp4` — while another worker is already
    // transferring a source file genuinely called `holiday-2.mp4` to exactly
    // that name. Sequentially one always sees the other; in parallel both
    // probe an empty slot and the loser's bytes are overwritten.
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_dest("holiday.mp4", b"already here, and different");
    rig.write_source("holiday.mp4", b"the renamed one");
    rig.write_source("holiday-2.mp4", b"a real file of that name");

    let mut resolver = FixedResolver(ConflictAction::Rename);
    let mut progress = SilentProgress;
    let summary = Transfer::new(
        &rig.link,
        &rig.source,
        &rig.destination,
        &rig.journal,
        &mut resolver,
        &mut progress,
    )
    .parallel(4)
    .run()
    .unwrap();

    assert_eq!(summary.transferred, 2);

    let mut found = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(rig.dest_dir.path()).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            found.insert(std::fs::read_to_string(&path).unwrap_or_default());
        }
    }
    for body in [
        "already here, and different",
        "the renamed one",
        "a real file of that name",
    ] {
        assert!(
            found.contains(body),
            "`{body}` was overwritten; the destination holds {found:?}"
        );
    }
}

#[test]
fn a_local_destination_is_not_asked_how_many_it_can_take() {
    // There is no per-client connection limit on a filesystem, so spending
    // handshakes to discover one would be pure waste.
    let rig = Rig::new(SourcePolicy::Delete);
    rig.write_source("a.mp4", b"x");

    let counted = CountingBackend {
        inner: Box::new(LocalBackend::new(rig.dest_dir.path().to_path_buf())),
        slot_enquiries: std::sync::atomic::AtomicUsize::new(0),
    };
    rig.run_over(&counted).unwrap();

    assert_eq!(
        counted.slot_enquiries.load(Ordering::SeqCst),
        0,
        "a local destination should never be probed for slots"
    );
}

/// Counts how often anything asks whether another connection is available.
struct CountingBackend {
    inner: Box<dyn Backend>,
    slot_enquiries: std::sync::atomic::AtomicUsize,
}

impl Backend for CountingBackend {
    fn capabilities(&self) -> tungstate_backend::Capabilities {
        self.inner.capabilities()
    }
    fn root_token(&self) -> tungstate_backend::Result<tungstate_backend::RootToken> {
        self.inner.root_token()
    }
    fn stat(&self, path: &Path) -> tungstate_backend::Result<tungstate_backend::Meta> {
        if path.to_string_lossy().starts_with(".tungstate-slot-") {
            self.slot_enquiries.fetch_add(1, Ordering::SeqCst);
        }
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
