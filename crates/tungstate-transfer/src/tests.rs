use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tungstate_backend::Backend;
use tungstate_backend::local::LocalBackend;
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
    fn stat(&self, path: &Path) -> tungstate_backend::Result<tungstate_backend::Meta> {
        self.0.stat(path)
    }
    fn read_dir(&self, path: &Path) -> tungstate_backend::Result<Vec<tungstate_backend::Entry>> {
        self.0.read_dir(path)
    }
    fn open_read(&self, path: &Path) -> tungstate_backend::Result<Box<dyn std::io::Read + Send>> {
        self.0.open_read(path)
    }
    fn create_write(&self, path: &Path) -> tungstate_backend::Result<Box<dyn Write + Send>> {
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

struct Corrupt(Box<dyn Write + Send>);

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
    let result = rig.run_over(&corrupting);

    assert!(result.is_err(), "a corrupted copy must not report success");
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
