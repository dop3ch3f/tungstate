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
        source: Endpoint::local("/Users/x/Videos"),
        destination: Endpoint::local("/Volumes/nas/inbox"),
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
    assert_eq!(link.destination.path, Path::new("/Volumes/nas/inbox"));
    assert_eq!(
        link.destination.connection, None,
        "an unqualified end is local"
    );
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
fn reopening_a_current_journal_preserves_its_rows() {
    // Renamed: this opens twice with current code, so it proves the migrations
    // are idempotent, not that an upgrade works. The genuine upgrade path is
    // `a_v3_journal_upgrades_to_v4_with_every_row_intact` below, which builds a
    // v3-shaped file with raw SQL because no v3 binary is around to build one.
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

/// A v3-shaped database, built with raw SQL because no v3 binary exists to
/// build one. Copied verbatim from migrations 1–3 and then frozen: if a later
/// migration edits history, this stops matching and the test says so.
fn write_v3_journal(path: &Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE ops (
             id          INTEGER PRIMARY KEY,
             kind        TEXT    NOT NULL,
             status      TEXT    NOT NULL,
             src_root    TEXT,
             src_path    TEXT,
             dst_root    TEXT,
             dst_path    TEXT,
             size        INTEGER,
             hash        TEXT,
             link        TEXT,
             note        TEXT,
             started_at  INTEGER NOT NULL,
             finished_at INTEGER
         );
         CREATE INDEX ops_src_path ON ops (src_path);
         CREATE INDEX ops_dst_path ON ops (dst_path);
         CREATE INDEX ops_hash     ON ops (hash) WHERE hash IS NOT NULL;
         CREATE INDEX ops_status   ON ops (status) WHERE status = 'intended';
         CREATE INDEX ops_started  ON ops (started_at);

         CREATE TABLE links (
             id             INTEGER PRIMARY KEY,
             name           TEXT    NOT NULL UNIQUE,
             source_root    TEXT    NOT NULL,
             dest_root      TEXT    NOT NULL,
             source_policy  TEXT    NOT NULL,
             verify         TEXT    NOT NULL,
             ordering       TEXT    NOT NULL,
             on_conflict    TEXT    NOT NULL,
             cooldown_secs  INTEGER NOT NULL,
             created_at     INTEGER NOT NULL
         );
         ALTER TABLE ops ADD COLUMN link_id INTEGER REFERENCES links (id);
         CREATE INDEX ops_link ON ops (link_id, status);

         ALTER TABLE links ADD COLUMN saved INTEGER NOT NULL DEFAULT 1;

         INSERT INTO links (
             id, name, source_root, dest_root, source_policy, verify,
             ordering, on_conflict, cooldown_secs, created_at, saved
         ) VALUES (
             1, 'laptop-to-nas', '/Users/x/Videos', '/Volumes/nas/inbox',
             'delete', 'hash', 'largest-first', 'quarantine', 30, 1700000000000, 1
         );
         INSERT INTO ops (
             id, kind, status, src_root, src_path, dst_root, dst_path,
             size, hash, link, link_id, started_at, finished_at
         ) VALUES (
             1, 'move', 'committed', '/Users/x/Videos', 'holiday.mp4',
             '/Volumes/nas/inbox', 'holiday.mp4', 4200, 'deadbeef',
             'laptop-to-nas', 1, 1700000000000, 1700000001000
         );",
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 3).unwrap();
}

#[test]
fn a_v3_journal_upgrades_to_v4_with_every_row_intact() {
    // The upgrade a user with an existing journal will actually take. The v4
    // columns are additive and NULL, which is what makes "every link written
    // before connections existed still means the local filesystem" true.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");
    write_v3_journal(&path);

    let journal = Journal::open(&path).unwrap();

    let links = journal.links().unwrap();
    assert_eq!(links.len(), 1, "the existing link must survive");
    assert_eq!(links[0].name, "laptop-to-nas");
    assert_eq!(links[0].source.path, Path::new("/Users/x/Videos"));
    assert_eq!(links[0].source.connection, None, "NULL means local");
    assert_eq!(links[0].destination.connection, None);
    assert_eq!(links[0].source_policy, SourcePolicy::Delete);

    let ops = journal.history(Path::new("holiday.mp4")).unwrap();
    assert_eq!(ops.len(), 1, "the existing op must survive");
    assert_eq!(ops[0].hash.as_deref(), Some("deadbeef"));
    assert_eq!(ops[0].source.as_ref().unwrap().connection, None);
    assert_eq!(ops[0].destination.as_ref().unwrap().connection, None);

    // And the full-path query still resolves, which is the branch v4 had to
    // narrow rather than extend.
    let found = journal
        .whereis(&Locator::Path(Path::new("/Volumes/nas/inbox/holiday.mp4")))
        .unwrap();
    assert_eq!(found.len(), 1);

    // Connections are now available on the upgraded file.
    assert!(journal.connections().unwrap().is_empty());
}

#[test]
fn a_journal_from_the_future_is_refused_rather_than_misread() {
    // Reversal of earlier policy, deliberate. A newer file can hold a link end
    // that is relative to a connection this build knows nothing about, and
    // reading that as a local path would drain into the wrong place.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");
    {
        let journal = Journal::open(&path).unwrap();
        journal.begin(&drain_op()).unwrap();
    }
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
    }

    match Journal::open(&path) {
        Err(JournalError::TooNew { found, known }) => {
            assert_eq!(found, 99);
            assert!(
                known >= 4,
                "this build should know at least v4, got {known}"
            );
        }
        other => panic!("expected TooNew, got {other:?}"),
    }
}

fn a_connection() -> NewConnection {
    NewConnection {
        name: "nas".to_string(),
        scheme: Scheme::Ftp,
        host: Some("nas.local".to_string()),
        port: Some(21),
        username: Some("me".to_string()),
        root: "/volume1/media".to_string(),
        options: std::collections::BTreeMap::from([("passive".to_string(), "true".to_string())]),
    }
}

#[test]
fn a_connection_round_trips_with_every_field_intact() {
    let journal = Journal::open_in_memory().unwrap();
    let id = journal.create_connection(&a_connection()).unwrap();

    let found = journal.connection_by_name("nas").unwrap();
    assert_eq!(found.id, id);
    assert_eq!(found.scheme, Scheme::Ftp);
    assert_eq!(found.host.as_deref(), Some("nas.local"));
    assert_eq!(found.port, Some(21));
    assert_eq!(found.username.as_deref(), Some("me"));
    assert_eq!(found.root, "/volume1/media");
    assert_eq!(
        found.options.get("passive").map(String::as_str),
        Some("true")
    );
    assert_eq!(journal.connection_by_id(id).unwrap(), found);
    assert_eq!(journal.connections().unwrap(), vec![found]);
}

#[test]
fn connection_names_are_unique() {
    let journal = Journal::open_in_memory().unwrap();
    journal.create_connection(&a_connection()).unwrap();

    let again = journal.create_connection(&a_connection());
    assert!(matches!(again, Err(JournalError::DuplicateConnection(name)) if name == "nas"));
}

#[test]
fn an_unknown_connection_name_is_an_error() {
    let journal = Journal::open_in_memory().unwrap();
    assert!(matches!(
        journal.connection_by_name("nope"),
        Err(JournalError::UnknownConnection(_))
    ));
    assert!(matches!(
        journal.delete_connection("nope"),
        Err(JournalError::UnknownConnection(_))
    ));
}

#[test]
fn a_connection_a_link_still_points_at_cannot_be_deleted() {
    // Enforced by the foreign key rather than by a check in Rust, so it holds
    // against any caller, including one that forgets to look.
    let journal = Journal::open_in_memory().unwrap();
    let id = journal.create_connection(&a_connection()).unwrap();
    journal
        .create_link(&NewLink {
            destination: Endpoint::remote(id, "inbox"),
            ..a_link()
        })
        .unwrap();

    assert!(matches!(
        journal.delete_connection("nas"),
        Err(JournalError::ConnectionInUse(name)) if name == "nas"
    ));
    assert_eq!(journal.connections().unwrap().len(), 1);
}

#[test]
fn an_unreferenced_connection_can_be_deleted() {
    let journal = Journal::open_in_memory().unwrap();
    journal.create_connection(&a_connection()).unwrap();

    journal.delete_connection("nas").unwrap();
    assert!(journal.connections().unwrap().is_empty());
}

#[test]
fn a_link_remembers_which_place_each_end_is() {
    let journal = Journal::open_in_memory().unwrap();
    let id = journal.create_connection(&a_connection()).unwrap();
    journal
        .create_link(&NewLink {
            destination: Endpoint::remote(id, "inbox/2026"),
            ..a_link()
        })
        .unwrap();

    let link = journal.link_by_name("laptop-to-nas").unwrap();
    assert_eq!(link.source.connection, None);
    assert_eq!(link.destination.connection, Some(id));
    assert_eq!(link.destination.path, Path::new("inbox/2026"));
}

#[test]
fn a_full_path_query_does_not_match_a_remote_row_by_concatenation() {
    // `root || '/' || path` is only a real path when the row is local. A
    // remote row splices a connection-relative root onto a
    // connection-relative path, and the result can collide with a genuine
    // local file. Both queries scope that branch to `connection IS NULL`.
    let journal = Journal::open_in_memory().unwrap();
    let id = journal.create_connection(&a_connection()).unwrap();

    let local = journal.begin(&drain_op()).unwrap();
    journal
        .finish(
            local,
            &Outcome::Committed {
                hash: Some("aaa".to_string()),
            },
        )
        .unwrap();

    // Same spelling as the local row, but on the far side of a connection.
    let remote = journal
        .begin(&NewOp {
            source: Some(Location {
                connection: Some(id),
                root: "/Users/x/Videos".into(),
                path: "holiday.mp4".into(),
            }),
            destination: None,
            ..drain_op()
        })
        .unwrap();
    journal
        .finish(
            remote,
            &Outcome::Committed {
                hash: Some("bbb".to_string()),
            },
        )
        .unwrap();

    let needle = Path::new("/Users/x/Videos/holiday.mp4");
    let history = journal.history(needle).unwrap();
    assert_eq!(
        history.iter().map(|o| o.id).collect::<Vec<_>>(),
        vec![local],
        "only the local row denotes that full path"
    );

    let located = journal.whereis(&Locator::Path(needle)).unwrap();
    assert_eq!(
        located.iter().map(|o| o.id).collect::<Vec<_>>(),
        vec![local]
    );

    // The relative spelling still finds both: that branch is not about roots.
    assert_eq!(journal.history(Path::new("holiday.mp4")).unwrap().len(), 2);
}
