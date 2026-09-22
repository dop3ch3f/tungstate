//! The executor, against real temp directories.
//!
//! The important test is `the_folder_ends_up_where_the_paper_model_said`: slice
//! 6's pure `Plan::apply_to` is used as this crate's specification, so one
//! assertion covers every op kind and every ordering the planner can produce.

use std::collections::BTreeSet;
use std::path::Path;

use tungstate_backend::local::LocalBackend;
use tungstate_core::plan::Plan;
use tungstate_core::{Policy, Snapshot};
use tungstate_journal::Journal;

use super::*;

/// A folder, its policy, and a journal, all thrown away at the end.
struct Fixture {
    dir: tempfile::TempDir,
    journal: Journal,
}

impl Fixture {
    fn new(policy: &str, files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join(".tungstate")).expect("dot dir");
        std::fs::write(dir.path().join(".tungstate/policy.toml"), policy).expect("policy");
        for (path, body) in files {
            let full = dir.path().join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("parent");
            }
            std::fs::write(full, body).expect("file");
        }
        Self {
            dir,
            journal: Journal::open_in_memory().expect("journal"),
        }
    }

    fn backend(&self) -> LocalBackend {
        LocalBackend::new(self.dir.path().to_path_buf())
    }

    fn root(&self) -> String {
        self.dir.path().to_string_lossy().to_string()
    }

    fn policy(&self) -> Policy {
        let text = std::fs::read_to_string(self.dir.path().join(".tungstate/policy.toml"))
            .expect("policy readable");
        Policy::parse(&text).expect("policy parses").policy
    }

    fn survey(&self) -> Snapshot {
        tungstate_attrs::survey(&self.backend(), &self.policy()).expect("survey")
    }

    fn plan(&self) -> Plan {
        self.policy().plan(&self.survey())
    }

    fn apply(&self) -> Result<Applied> {
        let plan = self.plan();
        let fresh = self.survey();
        apply(&plan, &fresh, &self.backend(), &self.journal, &self.root())
    }

    /// Every path under the root, so two shapes can be compared.
    fn shape(&self) -> Vec<String> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            let mut entries: Vec<_> = std::fs::read_dir(dir)
                .expect("readable")
                .filter_map(std::result::Result::ok)
                .collect();
            entries.sort_by_key(std::fs::DirEntry::path);
            for entry in entries {
                let relative = entry
                    .path()
                    .strip_prefix(base)
                    .expect("under base")
                    .to_string_lossy()
                    .replace('\\', "/");
                let meta = entry.metadata().expect("metadata");
                if meta.is_dir() {
                    out.push(format!("d {relative}"));
                    walk(&entry.path(), base, out);
                } else {
                    out.push(format!("f {relative} {}", meta.len()));
                }
            }
        }
        let mut out = Vec::new();
        walk(self.dir.path(), self.dir.path(), &mut out);
        out
    }
}

const BY_EXT: &str = "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"0s\"\n\n\
     [[rule]]\nname = \"text\"\npath = \"Text\"\nmatch = { ext = \"txt\" }\n\n\
     [[rule]]\nname = \"video\"\npath = \"Video\"\nmatch = { ext = \"mp4\" }\n";

/// The paths a snapshot holds, for comparing a prediction against reality.
fn paths(snapshot: &Snapshot) -> BTreeSet<String> {
    snapshot
        .entries
        .iter()
        .map(tungstate_core::Attributes::relative_path)
        .chain(snapshot.directories.iter().cloned())
        .collect()
}

// --- the specification ----------------------------------------------------

#[test]
fn the_folder_ends_up_where_the_paper_model_said() {
    // Slice 6's pure `apply_to` as this crate's specification. One assertion,
    // every op kind: moves, the directories they need, and the ones they empty.
    let fixture = Fixture::new(
        BY_EXT,
        &[
            ("a.txt", "one"),
            ("old/b.txt", "two"),
            ("old/deeper/c.mp4", "three"),
            ("keep.bin", "four"),
        ],
    );
    let before = fixture.survey();
    let plan = fixture.plan();
    let predicted = plan.apply_to(&before).expect("the plan is executable");

    let applied = fixture.apply().expect("apply succeeds");
    assert!(applied.complete(), "{applied:?}");

    let actual = fixture.survey();
    assert_eq!(
        paths(&actual),
        paths(&predicted),
        "the folder is not what the plan predicted"
    );
}

#[test]
fn a_second_plan_after_applying_finds_nothing_to_do() {
    // Convergence, on a real filesystem rather than on paper.
    let fixture = Fixture::new(BY_EXT, &[("a.txt", "one"), ("old/b.mp4", "two")]);
    fixture.apply().expect("apply succeeds");
    assert!(fixture.plan().ops.is_empty(), "{:?}", fixture.plan().ops);
}

#[test]
fn two_files_trading_places_really_do_trade_places() {
    // The temporary name, carried out for real.
    let policy = "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"0s\"\n\n\
         [[rule]]\nname = \"a\"\npath = \"\"\nrename = \"b.txt\"\nmatch = { glob = \"a.txt\" }\n\n\
         [[rule]]\nname = \"b\"\npath = \"\"\nrename = \"a.txt\"\nmatch = { glob = \"b.txt\" }\n";
    let fixture = Fixture::new(policy, &[("a.txt", "AAA"), ("b.txt", "BBB")]);
    fixture.apply().expect("apply succeeds");

    assert_eq!(
        std::fs::read_to_string(fixture.dir.path().join("a.txt")).unwrap(),
        "BBB"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.dir.path().join("b.txt")).unwrap(),
        "AAA"
    );
    // And no temporary left behind.
    assert!(
        !fixture.shape().iter().any(|p| p.contains("tungstate-swap")),
        "{:?}",
        fixture.shape()
    );
}

// --- undo -----------------------------------------------------------------

#[test]
fn undo_puts_the_folder_back_exactly_as_it_was() {
    let fixture = Fixture::new(
        BY_EXT,
        &[("a.txt", "one"), ("old/b.txt", "two"), ("c.mp4", "three")],
    );
    let before = fixture.shape();

    let applied = fixture.apply().expect("apply succeeds");
    assert_ne!(
        fixture.shape(),
        before,
        "apply should have changed something"
    );

    let undone = undo(
        applied.plan,
        &fixture.survey(),
        &fixture.backend(),
        &fixture.journal,
        &fixture.root(),
    )
    .expect("undo succeeds");
    assert_eq!(undone.done, applied.done);
    assert_eq!(fixture.shape(), before, "undo did not restore the shape");
}

#[test]
fn undo_is_refused_when_something_new_holds_the_old_name() {
    // Forcing would overwrite whatever has since been put there, and an undo
    // that destroys work is worse than no undo at all.
    let fixture = Fixture::new(BY_EXT, &[("a.txt", "one")]);
    let applied = fixture.apply().expect("apply succeeds");
    std::fs::write(fixture.dir.path().join("a.txt"), "something new").unwrap();

    let refused = undo(
        applied.plan,
        &fixture.survey(),
        &fixture.backend(),
        &fixture.journal,
        &fixture.root(),
    );
    assert!(
        matches!(refused, Err(ExecuteError::Stale { .. })),
        "{refused:?}"
    );
    // And refused as a whole: the file it did not want to overwrite is intact.
    assert_eq!(
        std::fs::read_to_string(fixture.dir.path().join("a.txt")).unwrap(),
        "something new"
    );
}

#[test]
fn undo_is_refused_when_the_file_is_not_where_it_was_left() {
    let fixture = Fixture::new(BY_EXT, &[("a.txt", "one")]);
    let applied = fixture.apply().expect("apply succeeds");
    std::fs::remove_file(fixture.dir.path().join("Text/a.txt")).unwrap();

    assert!(matches!(
        undo(
            applied.plan,
            &fixture.survey(),
            &fixture.backend(),
            &fixture.journal,
            &fixture.root()
        ),
        Err(ExecuteError::Stale { .. })
    ));
}

#[test]
fn a_plan_cannot_be_undone_twice() {
    let fixture = Fixture::new(BY_EXT, &[("a.txt", "one")]);
    let applied = fixture.apply().expect("apply succeeds");
    undo(
        applied.plan,
        &fixture.survey(),
        &fixture.backend(),
        &fixture.journal,
        &fixture.root(),
    )
    .expect("first undo");
    assert!(matches!(
        undo(
            applied.plan,
            &fixture.survey(),
            &fixture.backend(),
            &fixture.journal,
            &fixture.root()
        ),
        Err(ExecuteError::Journal(
            tungstate_journal::JournalError::PlanAlreadyUndone(_)
        ))
    ));
}

#[test]
fn an_undo_is_journalled_in_its_own_right() {
    // So it shows up in `log` like everything else, rather than history
    // quietly rewriting itself.
    let fixture = Fixture::new(BY_EXT, &[("a.txt", "one")]);
    let applied = fixture.apply().expect("apply succeeds");
    undo(
        applied.plan,
        &fixture.survey(),
        &fixture.backend(),
        &fixture.journal,
        &fixture.root(),
    )
    .expect("undo");

    let plans = fixture.journal.recent_plans(10).expect("plans");
    assert_eq!(plans.len(), 2, "the undo is a plan too");
    assert!(
        fixture
            .journal
            .plan_by_id(applied.plan)
            .unwrap()
            .is_undone()
    );
}

#[test]
fn inverting_reverses_the_order_as_well_as_the_operations() {
    // The part that is easy to get wrong: the last directory created has to be
    // the first one removed, or a parent is not empty when its turn comes.
    let journal = Journal::open_in_memory().expect("journal");
    let plan = journal.begin_plan("/root", "x", None).expect("plan");
    for op in [
        (OpKind::MkDir, Some("deep"), None),
        (OpKind::MkDir, Some("deep/deeper"), None),
        (OpKind::Rename, Some("a.txt"), Some("deep/deeper/a.txt")),
    ] {
        let id = journal
            .begin(&NewOp {
                kind: op.0,
                source: op.1.map(|p| Location::new("/root", p)),
                destination: op.2.map(|p| Location::new("/root", p)),
                size: None,
                link: None,
                link_id: None,
            })
            .expect("op");
        journal.attach_to_plan(id, plan).expect("attach");
        journal
            .finish(id, &Outcome::Committed { hash: None })
            .expect("finish");
    }

    let inverted = invert(&journal.ops_for_plan(plan).expect("ops"));
    assert_eq!(
        inverted,
        [
            Op::Move {
                from: "deep/deeper/a.txt".to_string(),
                to: "a.txt".to_string(),
                because: tungstate_core::plan::Because::Rule {
                    name: "undo".to_string()
                },
            },
            Op::RmDir {
                path: "deep/deeper".to_string()
            },
            Op::RmDir {
                path: "deep".to_string()
            },
        ]
    );
}

#[test]
fn inverting_ignores_what_never_happened() {
    // A failed or skipped operation changed nothing, so undoing it would be
    // inventing work.
    let journal = Journal::open_in_memory().expect("journal");
    let plan = journal.begin_plan("/root", "x", None).expect("plan");
    let failed = journal
        .begin(&NewOp {
            kind: OpKind::Rename,
            source: Some(Location::new("/root", "a.txt")),
            destination: Some(Location::new("/root", "Text/a.txt")),
            size: None,
            link: None,
            link_id: None,
        })
        .expect("op");
    journal.attach_to_plan(failed, plan).expect("attach");
    journal
        .finish(
            failed,
            &Outcome::Failed {
                error: "permission denied".to_string(),
            },
        )
        .expect("finish");

    assert!(invert(&journal.ops_for_plan(plan).expect("ops")).is_empty());
}

// --- staleness and failure ------------------------------------------------

#[test]
fn a_plan_is_refused_when_the_folder_has_moved_on() {
    // What makes a saved plan.json safe to apply later.
    let fixture = Fixture::new(BY_EXT, &[("a.txt", "one")]);
    let plan = fixture.plan();
    std::fs::write(fixture.dir.path().join("surprise.txt"), "new").unwrap();
    let fresh = fixture.survey();

    let refused = apply(
        &plan,
        &fresh,
        &fixture.backend(),
        &fixture.journal,
        &fixture.root(),
    );
    assert!(
        matches!(refused, Err(ExecuteError::Stale { .. })),
        "{refused:?}"
    );
    // Refused before anything moved.
    assert!(fixture.dir.path().join("a.txt").exists());
}

#[test]
fn a_file_written_between_planning_and_applying_is_skipped_not_moved() {
    // Somebody is editing it right now. Moving it out from under them is the
    // rudest thing this program could do.
    let fixture = Fixture::new(BY_EXT, &[("a.txt", "one"), ("b.txt", "two")]);
    let plan = fixture.plan();
    let mut fresh = fixture.survey();

    // Pretend `a.txt` was 99 bytes when planned, so the re-stat disagrees --
    // the same thing an editor saving the file would produce, without a sleep.
    for entry in &mut fresh.entries {
        if entry.relative_path() == "a.txt" {
            entry.size = 99;
        }
    }
    let plan = Plan {
        snapshot: tungstate_core::plan::fingerprint(&fresh),
        ..plan
    };

    let applied = apply(
        &plan,
        &fresh,
        &fixture.backend(),
        &fixture.journal,
        &fixture.root(),
    )
    .expect("apply runs");

    assert_eq!(applied.skipped.len(), 1, "{applied:?}");
    assert_eq!(applied.skipped[0].path, "a.txt");
    assert!(
        fixture.dir.path().join("a.txt").exists(),
        "left where it was"
    );
    assert!(
        fixture.dir.path().join("Text/b.txt").exists(),
        "the rest of the run carried on"
    );
}

#[test]
fn a_partly_applied_folder_is_described_correctly_by_the_next_plan() {
    // The convergence property paying for itself in the failure path: a run
    // that stops halfway leaves a folder the next plan gets right, so there is
    // never anything to resume.
    let fixture = Fixture::new(BY_EXT, &[("a.txt", "one"), ("b.txt", "two")]);
    let plan = fixture.plan();
    let fresh = fixture.survey();

    // Apply only the first half by hand.
    let half = Plan {
        ops: plan.ops.iter().take(2).cloned().collect(),
        snapshot: plan.snapshot.clone(),
        ..plan
    };
    apply(
        &half,
        &fresh,
        &fixture.backend(),
        &fixture.journal,
        &fixture.root(),
    )
    .expect("half applies");

    let next = fixture.plan();
    assert!(!next.ops.is_empty(), "there is still work to do");
    // And it is executable from where the folder actually is.
    assert!(next.apply_to(&fixture.survey()).is_ok());
}

// --- crash recovery -------------------------------------------------------

#[test]
fn an_interrupted_rename_is_resolved_by_asking_which_side_won() {
    let fixture = Fixture::new(BY_EXT, &[("a.txt", "one")]);
    let root = fixture.root();

    // A rename that was written down and then happened, with the process
    // dying before the outcome could be recorded.
    let op = fixture
        .journal
        .begin(&NewOp {
            kind: OpKind::Rename,
            source: Some(Location::new(&root, "a.txt")),
            destination: Some(Location::new(&root, "Text/a.txt")),
            size: Some(3),
            link: None,
            link_id: None,
        })
        .expect("op");
    std::fs::create_dir_all(fixture.dir.path().join("Text")).unwrap();
    std::fs::rename(
        fixture.dir.path().join("a.txt"),
        fixture.dir.path().join("Text/a.txt"),
    )
    .unwrap();

    let resolved =
        resolve_interrupted(&fixture.backend(), &fixture.journal, &root).expect("recovery");
    assert_eq!(resolved, [("a.txt".to_string(), Resolution::Committed)]);
    assert!(fixture.journal.incomplete().unwrap().is_empty());
    let _ = op;
}

#[test]
fn an_interrupted_rename_that_never_happened_is_recorded_as_abandoned() {
    let fixture = Fixture::new(BY_EXT, &[("a.txt", "one")]);
    let root = fixture.root();
    fixture
        .journal
        .begin(&NewOp {
            kind: OpKind::Rename,
            source: Some(Location::new(&root, "a.txt")),
            destination: Some(Location::new(&root, "Text/a.txt")),
            size: Some(3),
            link: None,
            link_id: None,
        })
        .expect("op");

    let resolved =
        resolve_interrupted(&fixture.backend(), &fixture.journal, &root).expect("recovery");
    assert_eq!(resolved, [("a.txt".to_string(), Resolution::Abandoned)]);
}

#[test]
fn a_drains_unfinished_work_is_left_for_the_transfer_engine() {
    // It may have left a half-copied temporary, which needs a repair rather
    // than a question. Not this crate's business.
    let fixture = Fixture::new(BY_EXT, &[("a.txt", "one")]);
    let root = fixture.root();
    let link = fixture
        .journal
        .create_link(&tungstate_journal::NewLink {
            name: "drain".to_string(),
            source: tungstate_journal::Endpoint::local(&root),
            destination: tungstate_journal::Endpoint::local("/elsewhere"),
            source_policy: tungstate_journal::SourcePolicy::Keep,
            verify: tungstate_journal::VerifyLevel::Size,
            order: tungstate_journal::Order::Discovered,
            on_conflict: tungstate_journal::ConflictAction::Skip,
            cooldown: std::time::Duration::ZERO,
            saved: true,
        })
        .expect("link");
    fixture
        .journal
        .begin(&NewOp {
            kind: OpKind::Move,
            source: Some(Location::new(&root, "a.txt")),
            destination: Some(Location::new("/elsewhere", "a.txt")),
            size: Some(3),
            link: Some("drain".to_string()),
            link_id: Some(link),
        })
        .expect("op");

    let resolved =
        resolve_interrupted(&fixture.backend(), &fixture.journal, &root).expect("recovery");
    assert!(resolved.is_empty(), "{resolved:?}");
    assert_eq!(fixture.journal.incomplete().unwrap().len(), 1);
}

// --- properties -----------------------------------------------------------
//
// Generated over folders and policies rather than hand-written ones, because
// the orderings that break an executor are the ones nobody thinks to build.

use proptest::prelude::*;

fn a_tree() -> impl Strategy<Value = Vec<(String, String)>> {
    proptest::collection::vec(
        (
            proptest::collection::vec("[a-c]{1,2}", 0..3),
            "[a-c]{1,2}",
            prop_oneof![Just("txt"), Just("mp4"), Just("bin")],
        ),
        0..6,
    )
    .prop_map(|raw| {
        let mut seen = BTreeSet::new();
        raw.into_iter()
            .filter_map(|(dirs, stem, ext)| {
                let mut path = dirs.join("/");
                if !path.is_empty() {
                    path.push('/');
                }
                path.push_str(&stem);
                path.push('.');
                path.push_str(ext);
                // A directory and a file cannot share a name, and a path
                // cannot appear twice.
                if seen.iter().any(|s: &String| {
                    s.starts_with(&format!("{path}/")) || path.starts_with(&format!("{s}/"))
                }) || !seen.insert(path.clone())
                {
                    return None;
                }
                Some((path, "x".to_string()))
            })
            .collect()
    })
}

fn a_policy() -> impl Strategy<Value = &'static str> {
    prop_oneof![
        Just(BY_EXT),
        Just(
            "[folder]\nname = \"f\"\ninbox = \"_inbox\"\n[defaults]\ncooldown = \"0s\"\n\n\
             [[rule]]\nname = \"by-ext\"\npath = \"{ext}\"\nmatch = { ext = [\"txt\", \"mp4\"] }\n"
        ),
        Just(
            "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"0s\"\non_conflict = \"rename\"\n\n\
             [[rule]]\nname = \"flat\"\npath = \"All\"\nrename = \"one.{ext}\"\n\
             match = { ext = [\"txt\", \"mp4\"] }\n"
        ),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// The executor does what the paper model said it would. One assertion
    /// covering every op kind and every ordering the planner can produce, and
    /// the reason `apply_to` was worth writing in slice 6.
    #[test]
    fn the_filesystem_agrees_with_the_paper_model(files in a_tree(), policy in a_policy()) {
        let fixture = Fixture::new(policy, &files.iter()
            .map(|(p, b)| (p.as_str(), b.as_str())).collect::<Vec<_>>());
        let before = fixture.survey();
        let plan = fixture.plan();
        prop_assume!(plan.settles);
        let predicted = plan.apply_to(&before).map_err(|e| TestCaseError::fail(e.to_string()))?;

        let applied = apply(&plan, &before, &fixture.backend(), &fixture.journal, &fixture.root())
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(applied.complete(), "{:?}", applied);
        prop_assert_eq!(paths(&fixture.survey()), paths(&predicted));
    }

    /// Apply then undo puts every file back where it started.
    #[test]
    fn undo_restores_the_folder(files in a_tree(), policy in a_policy()) {
        let fixture = Fixture::new(policy, &files.iter()
            .map(|(p, b)| (p.as_str(), b.as_str())).collect::<Vec<_>>());
        let before = fixture.shape();
        let plan = fixture.plan();
        prop_assume!(plan.settles);

        let applied = apply(&plan, &fixture.survey(), &fixture.backend(),
                            &fixture.journal, &fixture.root())
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assume!(applied.complete());

        undo(applied.plan, &fixture.survey(), &fixture.backend(),
             &fixture.journal, &fixture.root())
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(fixture.shape(), before);
    }
}

/// A policy that routes nothing, so a duplicate plan is the only thing acting.
fn inert() -> &'static str {
    "[folder]\nname = \"f\"\n[defaults]\ncooldown = \"0s\"\n"
}

#[test]
fn setting_duplicates_aside_can_be_undone() {
    // The regression this slice found: a file under the set-aside area is not
    // in any snapshot, because the walk is told never to enter it. Undo moves
    // files *out* of there, so the paper check has to take the journal's word
    // for them. Before this, no tidy that parked anything could be undone.
    let fixture = Fixture::new(
        inert(),
        &[("keep.bin", "same bytes"), ("copy/keep.bin", "same bytes")],
    );
    let snapshot = fixture.survey();
    let backend = fixture.backend();
    let mut digest = crate::digest::Cached::new(&backend, &fixture.journal, &fixture.root());
    let found = tungstate_core::dupes::find(
        &snapshot,
        &mut digest,
        &tungstate_core::dupes::Wants::default(),
    )
    .expect("the pass runs");
    assert_eq!(found.extra_files(), 1);

    let plan = tungstate_core::dupes::plan(
        &snapshot,
        &found,
        tungstate_core::dupes::Extras::SetAside,
        "f",
        tungstate_core::policy::Mode::Observe,
    );
    let applied = apply(
        &plan,
        &snapshot,
        &fixture.backend(),
        &fixture.journal,
        &fixture.root(),
    )
    .expect("it applies");

    let after = fixture.shape();
    let holds = |shape: &[String], path: &str| {
        shape
            .iter()
            .any(|entry| entry.starts_with(&format!("f {path} ")))
    };
    assert!(
        !holds(&after, "copy/keep.bin"),
        "the extra copy was set aside"
    );
    assert!(holds(&after, "keep.bin"), "one copy stays: {after:?}");
    assert!(
        holds(&after, ".tungstate-quarantine/copy/keep.bin"),
        "and it is in the set-aside area, not gone"
    );

    let fresh = fixture.survey();
    let undone = crate::undo::undo(
        applied.plan,
        &fresh,
        &fixture.backend(),
        &fixture.journal,
        &fixture.root(),
    )
    .expect("it can be taken back");
    assert!(undone.done > 0);
    assert!(
        holds(&fixture.shape(), "copy/keep.bin"),
        "and the copy is back where it was"
    );
}

#[test]
fn a_plan_that_used_the_trash_refuses_to_be_undone() {
    let fixture = Fixture::new(inert(), &[("one.bin", "same"), ("two.bin", "same")]);
    let snapshot = fixture.survey();
    let backend = fixture.backend();
    let mut digest = crate::digest::Cached::new(&backend, &fixture.journal, &fixture.root());
    let found = tungstate_core::dupes::find(
        &snapshot,
        &mut digest,
        &tungstate_core::dupes::Wants::default(),
    )
    .expect("the pass runs");
    let plan = tungstate_core::dupes::plan(
        &snapshot,
        &found,
        tungstate_core::dupes::Extras::Trash,
        "f",
        tungstate_core::policy::Mode::Observe,
    );

    // Recorded as irreversible before anything is carried out, so a crash
    // half-way cannot leave a record that claims otherwise.
    let id = fixture
        .journal
        .begin_plan_that(&fixture.root(), &plan.snapshot, None, false)
        .expect("recorded");
    let recorded = fixture.journal.plan_by_id(id).expect("readable");
    assert!(!recorded.reversible);

    let fresh = fixture.survey();
    let refused = crate::undo::undo(
        id,
        &fresh,
        &fixture.backend(),
        &fixture.journal,
        &fixture.root(),
    )
    .expect_err("it refuses");
    assert!(
        refused.to_string().contains("trash"),
        "the refusal says why: {refused}"
    );
}

#[test]
fn a_digest_is_read_once_and_remembered() {
    let fixture = Fixture::new(inert(), &[("one.bin", "same"), ("two.bin", "same")]);
    let snapshot = fixture.survey();
    let wants = tungstate_core::dupes::Wants::default();
    let backend = fixture.backend();

    let mut first = crate::digest::Cached::new(&backend, &fixture.journal, &fixture.root());
    tungstate_core::dupes::find(&snapshot, &mut first, &wants).expect("the pass runs");
    assert!(first.reads() > 0);
    assert_eq!(first.hits(), 0, "nothing was remembered yet");

    let mut again = crate::digest::Cached::new(&backend, &fixture.journal, &fixture.root());
    tungstate_core::dupes::find(&snapshot, &mut again, &wants).expect("the pass runs");
    assert_eq!(
        again.reads(),
        0,
        "a folder nobody touched is not read twice"
    );
    assert!(again.hits() > 0);
}
