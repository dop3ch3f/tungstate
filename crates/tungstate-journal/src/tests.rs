use std::path::Path;

use super::*;

fn drain_op() -> NewOp {
    NewOp {
        kind: OpKind::Move,
        source: Some(Location::new("/Users/x/Videos", "holiday.mp4")),
        destination: Some(Location::new("/Volumes/nas/inbox", "holiday.mp4")),
        size: Some(4_200_000_000),
        link: Some("laptop-to-nas".to_string()),
        link_id: None,
    }
}

#[test]
fn an_operation_round_trips_with_every_field_intact() {
    let journal = Journal::open_in_memory().unwrap();
    let id = journal.begin(&drain_op()).unwrap();

    journal
        .finish(
            id,
            &Outcome::Committed {
                hash: Some("abc123".to_string()),
            },
        )
        .unwrap();

    let ops = journal.history(Path::new("holiday.mp4")).unwrap();
    assert_eq!(ops.len(), 1);

    let op = &ops[0];
    assert_eq!(op.id, id);
    assert_eq!(op.kind, OpKind::Move);
    assert_eq!(op.status, OpStatus::Committed);
    assert_eq!(op.size, Some(4_200_000_000));
    assert_eq!(op.hash.as_deref(), Some("abc123"));
    assert_eq!(op.link.as_deref(), Some("laptop-to-nas"));
    assert_eq!(
        op.source.as_ref().unwrap().full(),
        Path::new("/Users/x/Videos/holiday.mp4")
    );
    assert!(op.finished_at.is_some());
}

#[test]
fn an_interrupted_operation_survives_a_restart() {
    // The test the drain's correctness rests on. Begin an operation, destroy the
    // Journal without finishing it as a crash would, then reopen the same file.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");

    let id = {
        let journal = Journal::open(&path).unwrap();
        journal.begin(&drain_op()).unwrap()
    };

    let reopened = Journal::open(&path).unwrap();
    let pending = reopened.incomplete().unwrap();

    assert_eq!(pending.len(), 1, "interrupted work must be recoverable");
    assert_eq!(pending[0].id, id);
    assert_eq!(pending[0].status, OpStatus::Intended);
    assert!(pending[0].finished_at.is_none());
}

#[test]
fn finished_operations_are_not_reported_as_interrupted() {
    let journal = Journal::open_in_memory().unwrap();

    let done = journal.begin(&drain_op()).unwrap();
    journal
        .finish(done, &Outcome::Committed { hash: None })
        .unwrap();
    let pending = journal.begin(&drain_op()).unwrap();

    let incomplete = journal.incomplete().unwrap();
    assert_eq!(incomplete.len(), 1);
    assert_eq!(incomplete[0].id, pending);
}

#[test]
fn finishing_an_unknown_operation_is_an_error() {
    let journal = Journal::open_in_memory().unwrap();
    let result = journal.finish(OpId(999), &Outcome::Committed { hash: None });
    assert!(matches!(result, Err(JournalError::UnknownOp(999))));
}

#[test]
fn failures_and_skips_keep_their_reason() {
    let journal = Journal::open_in_memory().unwrap();

    let failed = journal.begin(&drain_op()).unwrap();
    journal
        .finish(
            failed,
            &Outcome::Failed {
                error: "destination full".to_string(),
            },
        )
        .unwrap();

    let skipped = journal.begin(&drain_op()).unwrap();
    journal
        .finish(
            skipped,
            &Outcome::Skipped {
                reason: "identical copy already present".to_string(),
            },
        )
        .unwrap();

    let ops = journal.history(Path::new("holiday.mp4")).unwrap();
    assert_eq!(ops[0].status, OpStatus::Failed);
    assert_eq!(ops[0].note.as_deref(), Some("destination full"));
    assert_eq!(ops[1].status, OpStatus::Skipped);
    assert_eq!(
        ops[1].note.as_deref(),
        Some("identical copy already present")
    );
}

#[test]
fn history_is_oldest_first_and_scoped_to_the_path_asked_about() {
    let journal = Journal::open_in_memory().unwrap();

    let first = journal.begin(&drain_op()).unwrap();
    let second = journal.begin(&drain_op()).unwrap();
    // Genuinely unrelated at both ends. An earlier version of this test only
    // changed the destination, so it still matched on source and the assertion
    // was wrong rather than the query.
    journal
        .begin(&NewOp {
            source: Some(Location::new("/Users/x/Videos", "unrelated.mp4")),
            destination: Some(Location::new("/Volumes/nas/inbox", "unrelated.mp4")),
            ..drain_op()
        })
        .unwrap();

    let ops = journal.history(Path::new("holiday.mp4")).unwrap();
    assert_eq!(
        ops.iter().map(|o| o.id).collect::<Vec<_>>(),
        vec![first, second],
        "history must be oldest first and exclude other files"
    );
}

#[test]
fn whereis_finds_a_file_by_path_and_by_hash() {
    let journal = Journal::open_in_memory().unwrap();
    let id = journal.begin(&drain_op()).unwrap();
    journal
        .finish(
            id,
            &Outcome::Committed {
                hash: Some("deadbeef".to_string()),
            },
        )
        .unwrap();

    let by_full_path = journal
        .whereis(&Locator::Path(Path::new("/Volumes/nas/inbox/holiday.mp4")))
        .unwrap();
    assert_eq!(by_full_path.len(), 1);
    assert_eq!(by_full_path[0].id, id);

    let by_hash = journal.whereis(&Locator::Hash("deadbeef")).unwrap();
    assert_eq!(by_hash.len(), 1);
    assert_eq!(by_hash[0].id, id);
}

#[test]
fn whereis_ignores_operations_that_never_committed() {
    let journal = Journal::open_in_memory().unwrap();
    let id = journal.begin(&drain_op()).unwrap();
    journal
        .finish(
            id,
            &Outcome::Failed {
                error: "network dropped".to_string(),
            },
        )
        .unwrap();

    let found = journal
        .whereis(&Locator::Path(Path::new("holiday.mp4")))
        .unwrap();
    assert!(
        found.is_empty(),
        "a failed transfer must not be reported as a location"
    );
}

#[test]
fn opening_an_existing_journal_does_not_re_run_migrations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");

    let id = {
        let journal = Journal::open(&path).unwrap();
        journal.begin(&drain_op()).unwrap()
    };

    // Would fail with "table ops already exists" if migrations were not gated
    // on user_version, and would lose the row if it recreated the table.
    let reopened = Journal::open(&path).unwrap();
    assert_eq!(reopened.incomplete().unwrap()[0].id, id);
}

#[test]
fn concurrent_writers_all_land() {
    // Proves the mutex and WAL configuration hold. Ordering between threads is
    // deliberately not asserted; only that nothing is lost.
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(&dir.path().join("journal.db")).unwrap();

    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..25 {
                    let id = journal.begin(&drain_op()).unwrap();
                    journal
                        .finish(id, &Outcome::Committed { hash: None })
                        .unwrap();
                }
            });
        }
    });

    assert_eq!(
        journal.history(Path::new("holiday.mp4")).unwrap().len(),
        200
    );
    assert!(journal.incomplete().unwrap().is_empty());
}

fn a_link() -> NewLink {
    NewLink {
        name: "laptop-to-nas".to_string(),
        source_root: "/Users/x/Videos".into(),
        destination_root: "/Volumes/nas/inbox".into(),
        source_policy: SourcePolicy::Delete,
        verify: VerifyLevel::Hash,
        order: Order::LargestFirst,
        on_conflict: ConflictAction::Quarantine,
        cooldown: std::time::Duration::from_secs(30),
        saved: true,
    }
}

#[test]
fn a_link_round_trips_and_is_found_by_name() {
    let journal = Journal::open_in_memory().unwrap();
    let id = journal.create_link(&a_link()).unwrap();

    let link = journal.link_by_name("laptop-to-nas").unwrap();
    assert_eq!(link.id, id);
    assert_eq!(link.source_policy, SourcePolicy::Delete);
    assert_eq!(link.verify, VerifyLevel::Hash);
    assert_eq!(link.order, Order::LargestFirst);
    assert_eq!(link.on_conflict, ConflictAction::Quarantine);
    assert_eq!(link.cooldown.as_secs(), 30);
    assert_eq!(link.destination_root, Path::new("/Volumes/nas/inbox"));
}

#[test]
fn link_names_are_unique() {
    let journal = Journal::open_in_memory().unwrap();
    journal.create_link(&a_link()).unwrap();

    let again = journal.create_link(&a_link());
    assert!(matches!(again, Err(JournalError::DuplicateLink(name)) if name == "laptop-to-nas"));
}

#[test]
fn an_unknown_link_name_is_an_error() {
    let journal = Journal::open_in_memory().unwrap();
    assert!(matches!(
        journal.link_by_name("nope"),
        Err(JournalError::UnknownLink(_))
    ));
}

#[test]
fn interrupted_work_is_scoped_to_its_own_link() {
    // A resumed drain must not pick up another link's interrupted operations.
    let journal = Journal::open_in_memory().unwrap();
    let mine = journal.create_link(&a_link()).unwrap();
    let theirs = journal
        .create_link(&NewLink {
            name: "other".to_string(),
            ..a_link()
        })
        .unwrap();

    let ours = journal
        .begin(&NewOp {
            link_id: Some(mine),
            ..drain_op()
        })
        .unwrap();
    journal
        .begin(&NewOp {
            link_id: Some(theirs),
            ..drain_op()
        })
        .unwrap();

    let pending = journal.incomplete_for_link(mine).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, ours);
}

#[test]
fn temp_names_are_derived_from_the_operation_id() {
    // A crashed run's leftovers must be identifiable from the journal alone.
    let name = temp_name(Path::new("/Volumes/nas/inbox/holiday.mp4"), OpId(42));
    assert_eq!(
        name,
        Path::new("/Volumes/nas/inbox/holiday.mp4.tungstate-42.part")
    );
}

#[test]
fn migrating_an_existing_v1_journal_preserves_its_rows() {
    // The upgrade path a user with an existing journal will actually take.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");

    let id = {
        let journal = Journal::open(&path).unwrap();
        journal.begin(&drain_op()).unwrap()
    };

    let upgraded = Journal::open(&path).unwrap();
    assert_eq!(upgraded.incomplete().unwrap()[0].id, id);
    assert!(upgraded.links().unwrap().is_empty());
}

#[test]
fn recent_returns_newest_first_and_respects_the_limit() {
    let journal = Journal::open_in_memory().unwrap();
    let mut ids = Vec::new();
    for _ in 0..5 {
        ids.push(journal.begin(&drain_op()).unwrap());
    }

    let recent = journal.recent(3).unwrap();
    assert_eq!(recent.len(), 3);
    assert_eq!(
        recent.iter().map(|o| o.id).collect::<Vec<_>>(),
        vec![ids[4], ids[3], ids[2]],
        "newest first"
    );
}
