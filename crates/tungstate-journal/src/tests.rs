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
fn a_connection_update_replaces_every_mutable_field() {
    let journal = Journal::open_in_memory().unwrap();
    journal.create_connection(&a_connection()).unwrap();

    journal
        .update_connection(
            "nas",
            &ConnectionSettings {
                scheme: Scheme::Ftps,
                host: Some("nas.example".to_string()),
                port: Some(990),
                username: Some("someone-else".to_string()),
                root: "/volume2".to_string(),
                options: std::collections::BTreeMap::from([(
                    "passive".to_string(),
                    "false".to_string(),
                )]),
            },
        )
        .unwrap();

    let found = journal.connection_by_name("nas").unwrap();
    assert_eq!(found.scheme, Scheme::Ftps);
    assert_eq!(found.host.as_deref(), Some("nas.example"));
    assert_eq!(found.port, Some(990));
    assert_eq!(found.username.as_deref(), Some("someone-else"));
    assert_eq!(found.root, "/volume2");
    assert_eq!(
        found.options.get("passive").map(String::as_str),
        Some("false")
    );
}

#[test]
fn an_update_can_clear_a_field_that_was_set() {
    // The reason the API is a full replace: a partial patch over
    // `Option<Option<T>>` makes this case unreachable.
    let journal = Journal::open_in_memory().unwrap();
    journal.create_connection(&a_connection()).unwrap();

    let cleared = ConnectionSettings {
        port: None,
        username: None,
        options: std::collections::BTreeMap::new(),
        ..ConnectionSettings::from(&journal.connection_by_name("nas").unwrap())
    };
    journal.update_connection("nas", &cleared).unwrap();

    let found = journal.connection_by_name("nas").unwrap();
    assert_eq!(found.port, None);
    assert_eq!(found.username, None);
    assert!(found.options.is_empty());
}

#[test]
fn updating_an_unknown_connection_is_an_error() {
    let journal = Journal::open_in_memory().unwrap();
    journal.create_connection(&a_connection()).unwrap();
    let settings = ConnectionSettings::from(&journal.connection_by_name("nas").unwrap());

    assert!(matches!(
        journal.update_connection("nope", &settings),
        Err(JournalError::UnknownConnection(name)) if name == "nope"
    ));
}

#[test]
fn a_connection_a_link_points_at_can_still_be_edited() {
    // The asymmetry that makes editing safe: a link references a connection by
    // id, so moving the connection re-points every link at once. Deleting it
    // would leave those links pointing at nothing, which is why that is
    // refused and this is not.
    let journal = Journal::open_in_memory().unwrap();
    let id = journal.create_connection(&a_connection()).unwrap();
    journal
        .create_link(&NewLink {
            destination: Endpoint::remote(id, "inbox"),
            ..a_link()
        })
        .unwrap();

    let moved = ConnectionSettings {
        root: "/volume2/media".to_string(),
        ..ConnectionSettings::from(&journal.connection_by_name("nas").unwrap())
    };
    journal.update_connection("nas", &moved).unwrap();

    let link = journal.link_by_name("laptop-to-nas").unwrap();
    assert_eq!(link.destination.connection, Some(id));
    assert_eq!(
        journal.connection_by_id(id).unwrap().root,
        "/volume2/media",
        "the link still resolves, and now resolves somewhere else"
    );
}

#[test]
fn the_links_blocking_a_delete_can_be_named() {
    let journal = Journal::open_in_memory().unwrap();
    let id = journal.create_connection(&a_connection()).unwrap();
    journal
        .create_link(&NewLink {
            name: "drain-to-nas".to_string(),
            destination: Endpoint::remote(id, "inbox"),
            ..a_link()
        })
        .unwrap();
    journal
        .create_link(&NewLink {
            name: "back-from-nas".to_string(),
            source: Endpoint::remote(id, "outbox"),
            ..a_link()
        })
        .unwrap();

    // Both ends count, and the order is stable so a message reads the same
    // way twice.
    assert_eq!(
        journal.links_using(id).unwrap(),
        vec!["back-from-nas".to_string(), "drain-to-nas".to_string()]
    );
}

#[test]
fn an_unused_connection_has_no_links_to_name() {
    let journal = Journal::open_in_memory().unwrap();
    let id = journal.create_connection(&a_connection()).unwrap();
    assert!(journal.links_using(id).unwrap().is_empty());
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

#[test]
fn a_remote_location_reads_with_the_separator_the_far_side_uses() {
    // Caught by Windows CI. `Location::full` goes through `Path::join`, which
    // produces `inbox\a.mp4` there — a name no FTP or S3 server has ever
    // heard of, printed by `whereis` for someone to copy.
    let remote = Location {
        connection: Some(ConnectionId(1)),
        root: "inbox".into(),
        path: "2026/a.mp4".into(),
    };
    assert_eq!(remote.display_path(), "inbox/2026/a.mp4");

    // The root of a connection, where one half is empty.
    let at_root = Location {
        connection: Some(ConnectionId(1)),
        root: PathBuf::new(),
        path: "a.mp4".into(),
    };
    assert_eq!(at_root.display_path(), "a.mp4");

    // A local one still reads as a path on this machine, separators and all.
    let local = Location::new("/Users/x", "a.mp4");
    assert_eq!(
        local.display_path(),
        Path::new("/Users/x").join("a.mp4").display().to_string()
    );
}

#[test]
fn a_selection_round_trips_and_replaces_rather_than_accumulates() {
    let journal = Journal::open_in_memory().unwrap();
    let id = journal.create_link(&a_link()).unwrap();

    assert!(
        journal.files_for(id).unwrap().is_empty(),
        "no selection means the whole source root"
    );

    journal
        .set_files(id, &[PathBuf::from("a.mp4"), PathBuf::from("b.mp4")])
        .unwrap();
    assert_eq!(
        journal.files_for(id).unwrap(),
        vec![PathBuf::from("a.mp4"), PathBuf::from("b.mp4")]
    );

    // Setting again replaces; it does not append.
    journal.set_files(id, &[PathBuf::from("c.mp4")]).unwrap();
    assert_eq!(journal.files_for(id).unwrap(), vec![PathBuf::from("c.mp4")]);

    // And the same path twice is one queue entry, not two copies of the file.
    journal
        .set_files(id, &[PathBuf::from("d.mp4"), PathBuf::from("d.mp4")])
        .unwrap();
    assert_eq!(journal.files_for(id).unwrap(), vec![PathBuf::from("d.mp4")]);

    journal.set_files(id, &[]).unwrap();
    assert!(journal.files_for(id).unwrap().is_empty());
}

#[test]
fn a_selection_belongs_to_its_own_link() {
    let journal = Journal::open_in_memory().unwrap();
    let mine = journal.create_link(&a_link()).unwrap();
    let theirs = journal
        .create_link(&NewLink {
            name: "other".to_string(),
            ..a_link()
        })
        .unwrap();

    journal
        .set_files(mine, &[PathBuf::from("mine.mp4")])
        .unwrap();
    assert_eq!(journal.files_for(mine).unwrap().len(), 1);
    assert!(journal.files_for(theirs).unwrap().is_empty());
}

#[test]
fn a_v4_journal_upgrades_to_v5_and_its_links_mean_the_whole_source() {
    // The upgrade a user of v0.1.0-alpha.1 will take. Their links predate
    // selections, and an empty selection is exactly what they already meant.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");
    write_v3_journal(&path);
    {
        // v4 on top of the v3 fixture, as the released build would have left it.
        let journal = Journal::open(&path).unwrap();
        assert_eq!(journal.links().unwrap().len(), 1);
    }

    let journal = Journal::open(&path).unwrap();
    let links = journal.links().unwrap();
    assert_eq!(links.len(), 1, "the existing link must survive");
    assert!(
        journal.files_for(links[0].id).unwrap().is_empty(),
        "and mean the whole source root, as it always did"
    );

    // The new table is usable on the upgraded file.
    journal
        .set_files(links[0].id, &[PathBuf::from("a.mp4")])
        .unwrap();
    assert_eq!(journal.files_for(links[0].id).unwrap().len(), 1);
}

#[test]
fn a_networked_connection_with_no_root_is_warned_about() {
    // The trap this exists for: OpenDAL normalises an empty root to `/`, so
    // the drain writes relative to the server's own root and every file is
    // refused at the far side, long after the mistake was made.
    let warning = Scheme::Ftp
        .rootless_warning("")
        .expect("ftp with no root warns");
    assert!(warning.contains("server's own `/`"), "{warning}");
    assert!(warning.contains("connection test"), "{warning}");

    // Whitespace is no root either.
    assert!(Scheme::Ftps.rootless_warning("   ").is_some());

    // A root that was given is not warned about, whatever it is — `/` may
    // genuinely be right on a server that chroots the login.
    assert!(Scheme::Ftp.rootless_warning("/volume1/media").is_none());
    assert!(Scheme::Ftp.rootless_warning("/").is_none());

    // A local filesystem connection has no far side to be wrong about, and
    // the factory refuses a root that is not a directory anyway.
    assert!(Scheme::Fs.rootless_warning("").is_none());
}

#[test]
fn two_journals_on_one_file_do_not_collide() {
    // Two drains at once is an ordinary thing to want, and each is its own
    // process with its own connection. WAL still allows only one writer, so
    // without a busy timeout the second gets SQLITE_BUSY the instant it
    // collides rather than waiting for the first to commit.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");
    let one = Journal::open(&path).expect("first journal");
    let two = Journal::open(&path).expect("second journal");

    std::thread::scope(|scope| {
        for journal in [&one, &two] {
            scope.spawn(move || {
                for _ in 0..50 {
                    journal
                        .begin(&drain_op())
                        .expect("a write should wait for the other writer, not fail");
                }
            });
        }
    });

    assert_eq!(one.recent(1000).unwrap().len(), 100);
}

/// A link with the given name, and whether it has ever run.
fn link_named(journal: &Journal, name: &str) -> LinkId {
    journal
        .create_link(&NewLink {
            name: name.to_string(),
            source: Endpoint::local("/Users/x/Videos"),
            destination: Endpoint::local("/Volumes/nas/inbox"),
            source_policy: SourcePolicy::Delete,
            verify: VerifyLevel::Hash,
            order: Order::LargestFirst,
            on_conflict: ConflictAction::Quarantine,
            cooldown: std::time::Duration::from_secs(30),
            saved: true,
        })
        .expect("link")
}

#[test]
fn a_link_that_never_ran_is_deleted_outright() {
    let journal = Journal::open_in_memory().unwrap();
    link_named(&journal, "fresh");

    assert_eq!(journal.remove_link("fresh").unwrap(), Removal::Deleted);
    assert!(journal.links().unwrap().is_empty());
    // The name is free, which is the whole point of deleting rather than
    // retiring when nothing refers to the row.
    link_named(&journal, "fresh");
    assert_eq!(journal.links().unwrap().len(), 1);
}

#[test]
fn a_link_with_history_is_retired_so_its_history_still_resolves() {
    let journal = Journal::open_in_memory().unwrap();
    let id = link_named(&journal, "nas");
    let op = journal
        .begin(&NewOp {
            link_id: Some(id),
            ..drain_op()
        })
        .unwrap();
    journal
        .finish(op, &Outcome::Committed { hash: None })
        .unwrap();

    assert_eq!(journal.remove_link("nas").unwrap(), Removal::Retired);

    // Gone from everything that offers a link to run.
    assert!(journal.links().unwrap().is_empty());
    assert!(journal.link_by_name("nas").is_err());

    // But the operation still knows what it belonged to, which is the reason
    // the row is kept at all: history that outlives its link and stops making
    // sense is worse than not being able to remove one.
    let still_there = journal
        .link_by_id(id)
        .expect("a retired link resolves by id");
    assert!(still_there.deleted_at.is_some());

    // And the name is released, so it can be used again.
    link_named(&journal, "nas");
    assert_eq!(journal.links().unwrap().len(), 1);
}

#[test]
fn a_link_with_unfinished_work_is_refused_rather_than_removed() {
    // Removing it would strand a part-copied file at the destination with
    // nothing left able to name it.
    let journal = Journal::open_in_memory().unwrap();
    let id = link_named(&journal, "busy");
    journal
        .begin(&NewOp {
            link_id: Some(id),
            ..drain_op()
        })
        .unwrap();

    let refused = journal.remove_link("busy").unwrap_err();
    let message = refused.to_string();
    assert!(message.contains("unfinished"), "{message}");
    assert!(message.contains("link discard busy"), "{message}");
    assert!(journal.link_by_name("busy").is_ok(), "it is still there");
}

#[test]
fn removing_a_link_that_is_not_there_says_so() {
    let journal = Journal::open_in_memory().unwrap();
    assert!(journal.remove_link("nothing").is_err());

    // And a retired one is not there any more either.
    let id = link_named(&journal, "once");
    let op = journal
        .begin(&NewOp {
            link_id: Some(id),
            ..drain_op()
        })
        .unwrap();
    journal
        .finish(op, &Outcome::Committed { hash: None })
        .unwrap();
    journal.remove_link("once").unwrap();
    assert!(journal.remove_link("once").is_err());
}

#[test]
fn a_path_is_found_by_the_spelling_this_platform_uses() {
    // Windows caught this: a row keeps its root and its path apart, the
    // queries join them with `/`, and every caller on Windows types
    // backslashes throughout — so nothing ever matched and `tungstate log`
    // against an absolute path had never worked there.
    //
    // Built with `PathBuf` rather than a literal, so the test asks the
    // question in whichever spelling the platform actually produces.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("dst");
    let full = root.join("a.mp4");

    let journal = Journal::open_in_memory().unwrap();
    let id = journal
        .begin(&NewOp {
            source: Some(Location::new(dir.path().join("src"), "a.mp4")),
            destination: Some(Location::new(&root, "a.mp4")),
            ..drain_op()
        })
        .unwrap();
    journal
        .finish(id, &Outcome::Committed { hash: None })
        .unwrap();

    assert_eq!(
        journal.history(&full).unwrap().len(),
        1,
        "an absolute path should find its own operation"
    );
    assert_eq!(
        journal.whereis(&Locator::Path(&full)).unwrap().len(),
        1,
        "and so should whereis"
    );

    // A relative path still matches the stored path on its own.
    assert_eq!(journal.history(Path::new("a.mp4")).unwrap().len(), 1);
    // And something else does not.
    assert!(journal.history(&root.join("b.mp4")).unwrap().is_empty());
}

#[test]
fn windows_spellings_are_normalised_and_unix_ones_are_left_alone() {
    use crate::{path_clause, separators_as_slashes};

    // Both branches, from whichever platform is running this. The Windows one
    // was wrong for the life of the project and only CI could see it.
    assert_eq!(
        separators_as_slashes(r"C:\Users\me\dst\a.mp4", true),
        "C:/Users/me/dst/a.mp4"
    );
    assert_eq!(
        separators_as_slashes("/Users/me/dst/a.mp4", true),
        "/Users/me/dst/a.mp4"
    );

    // On Unix a backslash is an ordinary character in a filename, so leaving
    // it alone is the difference between finding one file and conflating two.
    assert_eq!(
        separators_as_slashes(r"odd\name.mp4", false),
        r"odd\name.mp4"
    );

    // The clause normalises the stored side to match, and only where it must.
    let windows = path_clause(true);
    assert!(windows.contains("replace(src_path"), "{windows}");
    assert!(windows.contains(r"'\', '/'"), "{windows}");
    let unix = path_clause(false);
    assert!(!unix.contains("replace("), "{unix}");
    assert!(unix.contains("src_root || '/' || src_path"), "{unix}");

    // And both spellings are SQL this database will actually accept. A syntax
    // error in the Windows branch is otherwise invisible from anywhere but a
    // Windows CI run, which is a slow way to find a typo.
    let conn = rusqlite::Connection::open_in_memory().expect("memory database");
    crate::schema::prepare(&conn).expect("schema");
    for (platform, clause) in [("windows", &windows), ("unix", &unix)] {
        conn.prepare(&format!("SELECT * FROM ops WHERE {clause}"))
            .unwrap_or_else(|e| panic!("the {platform} clause is not valid SQL: {e}\n{clause}"));
    }
}

// ---------------------------------------------------------------------------
// Export, import, archive and reset

/// A journal with something in every table, so a round trip has work to do.
fn populated(path: &std::path::Path) -> Journal {
    let journal = Journal::open(path).expect("journal");
    let connection = journal
        .create_connection(&NewConnection {
            name: "nas".to_string(),
            scheme: Scheme::Ftp,
            host: Some("192.168.1.2".to_string()),
            port: Some(21),
            username: Some("me".to_string()),
            root: "/volume1".to_string(),
            options: std::collections::BTreeMap::from([("passive".into(), "true".into())]),
        })
        .expect("connection");
    let link = journal
        .create_link(&NewLink {
            name: "drain".to_string(),
            source: Endpoint::local("/Users/me/Videos"),
            destination: Endpoint::remote(connection, std::path::PathBuf::from("inbox")),
            source_policy: SourcePolicy::Delete,
            verify: VerifyLevel::Readback,
            order: Order::OldestFirst,
            on_conflict: ConflictAction::Rename,
            cooldown: std::time::Duration::from_secs(45),
            saved: true,
        })
        .expect("link");
    journal
        .set_files(link, &[std::path::PathBuf::from("a.mp4")])
        .expect("selection");
    let op = journal
        .begin(&NewOp {
            link_id: Some(link),
            ..drain_op()
        })
        .expect("op");
    journal
        .finish(
            op,
            &Outcome::Committed {
                hash: Some("abc".into()),
            },
        )
        .expect("finish");
    // A reorganisation as well as a drain, so the fixture is a journal that
    // has done both of the things tungstate does.
    let plan = journal
        .begin_plan("/Users/me/Downloads", "fingerprint")
        .expect("plan");
    let tidy = journal
        .begin(&NewOp {
            kind: OpKind::Rename,
            source: Some(Location::new("/Users/me/Downloads", "a.txt")),
            destination: Some(Location::new("/Users/me/Downloads", "Text/a.txt")),
            size: Some(12),
            link: None,
            link_id: None,
        })
        .expect("op");
    journal.attach_to_plan(tidy, plan).expect("attach");
    journal
        .finish(tidy, &Outcome::Committed { hash: None })
        .expect("finish");
    journal
}

#[test]
fn an_export_carries_every_column_the_database_has() {
    // Reflection in the implementation rather than a hand-written list, so a
    // column added in some later slice cannot be silently left out of an
    // archive. This test is what says so out loud.
    let dir = tempfile::tempdir().unwrap();
    let journal = populated(&dir.path().join("journal.db"));
    let document = journal.export().unwrap();

    for table in ["ops", "links", "connections", "link_files", "plans"] {
        let columns: Vec<String> = {
            let conn = journal.lock();
            let statement = conn
                .prepare(&format!("SELECT * FROM {table} LIMIT 0"))
                .unwrap();
            statement
                .column_names()
                .into_iter()
                .map(str::to_string)
                .collect()
        };
        let row = document[table]
            .as_array()
            .and_then(|rows| rows.first())
            .unwrap_or_else(|| panic!("`{table}` should have a row to compare"));
        for column in columns {
            assert!(
                row.get(&column).is_some(),
                "`{table}.{column}` is in the database and missing from the export"
            );
        }
    }
    assert_eq!(document["tungstate_export"], crate::storage::EXPORT_VERSION);
}

#[test]
fn an_export_carries_no_passwords() {
    // They live in the keychain under a key derived from the connection name,
    // and an archive is a file that gets copied around.
    let dir = tempfile::tempdir().unwrap();
    let journal = populated(&dir.path().join("journal.db"));
    let text = serde_json::to_string(&journal.export().unwrap()).unwrap();
    for forbidden in ["password", "secret", "passwd"] {
        assert!(
            !text.contains(forbidden),
            "`{forbidden}` appears in an export"
        );
    }
}

#[test]
fn a_journal_survives_a_round_trip_through_json() {
    let dir = tempfile::tempdir().unwrap();
    let original = populated(&dir.path().join("journal.db"));
    let document = original.export().unwrap();

    let restored = Journal::open(&dir.path().join("other.db")).unwrap();
    let loaded = restored.import(&document).unwrap();
    assert!(loaded >= 4, "every table should have contributed a row");

    // The parts a person would notice, by value rather than by id.
    let before = original.links().unwrap();
    let after = restored.links().unwrap();
    assert_eq!(before.len(), after.len());
    assert_eq!(before[0].name, after[0].name);
    assert_eq!(before[0].verify, after[0].verify);
    assert_eq!(before[0].order, after[0].order);
    assert_eq!(before[0].cooldown, after[0].cooldown);
    assert_eq!(before[0].destination, after[0].destination);

    // The reference that would break if ids were not preserved.
    let op = restored.recent(10).unwrap().pop().unwrap();
    let link = restored
        .link_by_id(op.link_id.expect("the op knows its link"))
        .unwrap();
    assert_eq!(link.name, "drain");

    // And the selection, which lives in its own table.
    assert_eq!(restored.files_for(after[0].id).unwrap().len(), 1);
    assert_eq!(
        restored
            .connection_by_name("nas")
            .unwrap()
            .options
            .get("passive"),
        Some(&"true".to_string())
    );
}

#[test]
fn an_import_refuses_to_merge_into_a_journal_that_holds_something() {
    let dir = tempfile::tempdir().unwrap();
    let original = populated(&dir.path().join("journal.db"));
    let document = original.export().unwrap();

    // Into itself: there is already a link and a connection there.
    let refused = original.import(&document).unwrap_err();
    assert!(matches!(refused, JournalError::NotEmpty), "{refused}");
    assert_eq!(original.links().unwrap().len(), 1, "nothing was touched");
}

#[test]
fn a_document_from_somewhere_else_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(&dir.path().join("journal.db")).unwrap();
    let error = journal
        .import(&serde_json::json!({ "links": [] }))
        .unwrap_err();
    assert!(matches!(error, JournalError::BadExport(_)), "{error}");

    // And one from a future tungstate says so rather than half-reading it.
    let ahead = serde_json::json!({ "tungstate_export": crate::storage::EXPORT_VERSION + 1 });
    let error = journal.import(&ahead).unwrap_err();
    assert!(error.to_string().contains("newer tungstate"), "{error}");
}

#[test]
fn a_reset_archives_first_and_the_archive_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let journal = populated(&dir.path().join("journal.db"));

    let name = journal.reset().unwrap();
    assert!(journal.links().unwrap().is_empty(), "the slate is clean");
    assert!(journal.connections().unwrap().is_empty());

    let archives = journal.archives().unwrap();
    assert_eq!(archives.len(), 1);
    assert_eq!(archives[0].name, name);
    let summary = archives[0].summary.expect("an archive describes itself");
    assert_eq!(summary.links, 1);
    assert_eq!(summary.connections, 1);
    // A drain's operation and a reorganisation's, since the fixture does both.
    assert_eq!(summary.operations, 2);
    assert_eq!(summary.plans, 1);

    // Restoring puts the empty one aside in its turn, so it is reversible.
    // The two names must differ even though both are written in the same
    // second: an archive that overwrites another is the one way this could
    // lose something.
    let put_aside = journal.restore(&name).unwrap();
    assert_ne!(
        put_aside, name,
        "the restore overwrote the archive it restored"
    );
    assert_eq!(journal.archives().unwrap().len(), 2, "both are still there");
    assert_eq!(journal.links().unwrap().len(), 1);
    assert_eq!(journal.links().unwrap()[0].name, "drain");
}

#[test]
fn a_reset_is_refused_while_a_run_left_work_behind() {
    // Those rows are the only thing that knows a part-copied file exists at
    // the far end.
    let dir = tempfile::tempdir().unwrap();
    let journal = populated(&dir.path().join("journal.db"));
    journal.begin(&drain_op()).unwrap();

    let refused = journal.reset().unwrap_err();
    assert!(
        matches!(refused, JournalError::Unfinished { .. }),
        "{refused}"
    );
    assert!(refused.to_string().contains("link unfinished"), "{refused}");
    assert_eq!(journal.links().unwrap().len(), 1, "nothing was touched");
    assert!(
        journal.archives().unwrap().is_empty(),
        "and nothing was written"
    );
}

#[test]
fn an_archive_name_cannot_reach_outside_its_directory() {
    // The name arrives from a window or a command line and is joined onto a
    // directory, so it is untrusted input.
    let dir = tempfile::tempdir().unwrap();
    let journal = populated(&dir.path().join("journal.db"));
    for hostile in [
        "../../etc/passwd.json",
        "/etc/passwd.json",
        "a/b.json",
        "plain.txt",
    ] {
        assert!(
            journal.restore(hostile).is_err(),
            "`{hostile}` was accepted"
        );
        assert!(
            journal.forget_archive(hostile).is_err(),
            "`{hostile}` was accepted"
        );
    }
}

#[test]
fn an_archive_can_be_forgotten_for_good() {
    let dir = tempfile::tempdir().unwrap();
    let journal = populated(&dir.path().join("journal.db"));
    let name = journal.reset().unwrap();

    journal.forget_archive(&name).unwrap();
    assert!(journal.archives().unwrap().is_empty());
    assert!(journal.forget_archive(&name).is_err(), "and it is gone");
}

#[test]
fn an_in_memory_journal_says_it_has_nowhere_to_archive() {
    let journal = Journal::open_in_memory().unwrap();
    assert!(matches!(
        journal.reset().unwrap_err(),
        JournalError::NotOnDisk
    ));
    // Exporting still works: it is the file that is missing, not the data.
    assert!(journal.export().is_ok());
}

#[test]
fn only_quarantine_is_a_question() {
    // The rule both surfaces read, so they cannot disagree about one setting.
    // Every prompt holds a worker until it is answered, so asking about a
    // decision somebody already made turns an unattended drain into one that
    // stops at the first clash and waits.
    assert!(
        ConflictAction::Quarantine.wants_asking(),
        "the one labelled ask me"
    );
    for decided in [
        ConflictAction::Rename,
        ConflictAction::Skip,
        ConflictAction::Replace,
    ] {
        assert!(
            !decided.wants_asking(),
            "{decided:?} is a decision, not a question"
        );
    }
}

// --- plans (v7) -----------------------------------------------------------

use crate::plans::PlanId;

/// A journal with one applied plan carrying two operations.
fn with_plan(journal: &Journal) -> PlanId {
    let plan = journal
        .begin_plan("/Users/me/Downloads", "abc123")
        .expect("plan");
    for (from, to) in [("a.txt", "Text/a.txt"), ("b.txt", "Text/b.txt")] {
        let op = journal
            .begin(&NewOp {
                kind: OpKind::Rename,
                source: Some(Location::new("/Users/me/Downloads", from)),
                destination: Some(Location::new("/Users/me/Downloads", to)),
                size: Some(10),
                link: None,
                link_id: None,
            })
            .expect("op");
        journal.attach_to_plan(op, plan).expect("attach");
        journal
            .finish(op, &Outcome::Committed { hash: None })
            .expect("finish");
    }
    plan
}

#[test]
fn a_plan_collects_the_operations_that_belong_to_it() {
    let journal = Journal::open_in_memory().expect("journal");
    let plan = with_plan(&journal);
    let ops = journal.ops_for_plan(plan).expect("ops");
    assert_eq!(ops.len(), 2);
    // Oldest first, which is the order it was applied in -- so reversing it is
    // what undo wants and `.rev()` is the whole of the difference.
    assert_eq!(
        ops[0].source.as_ref().unwrap().display_path(),
        "/Users/me/Downloads/a.txt"
    );
}

#[test]
fn a_drains_operations_belong_to_no_plan() {
    // `plan_id IS NULL` reads as "a drain did this, not a reorganisation",
    // which is what makes the migration additive and every existing row right.
    let journal = Journal::open_in_memory().expect("journal");
    let op = journal
        .begin(&NewOp {
            kind: OpKind::Move,
            source: Some(Location::new("/src", "a.mp4")),
            destination: Some(Location::new("/dst", "a.mp4")),
            size: Some(1),
            link: None,
            link_id: None,
        })
        .expect("op");
    journal
        .finish(op, &Outcome::Committed { hash: None })
        .expect("finish");
    let plan = journal.begin_plan("/elsewhere", "zzz").expect("plan");
    assert!(journal.ops_for_plan(plan).expect("ops").is_empty());
}

#[test]
fn recent_plans_are_newest_first_and_include_undone_ones() {
    // "Undo the last three" has to be able to say *that one is already back*
    // rather than silently counting past it.
    let journal = Journal::open_in_memory().expect("journal");
    let first = journal.begin_plan("/a", "1").expect("plan");
    let second = journal.begin_plan("/b", "2").expect("plan");
    journal.mark_undone(first).expect("undo");

    let recent = journal.recent_plans(10).expect("recent");
    assert_eq!(
        recent.iter().map(|p| p.id).collect::<Vec<_>>(),
        [second, first]
    );
    assert!(recent[1].is_undone());
    assert!(!recent[0].is_undone());
}

#[test]
fn recent_plans_honours_its_limit() {
    let journal = Journal::open_in_memory().expect("journal");
    for n in 0..5 {
        journal.begin_plan(&format!("/{n}"), "x").expect("plan");
    }
    assert_eq!(journal.recent_plans(2).expect("recent").len(), 2);
}

#[test]
fn a_plan_cannot_be_undone_twice() {
    let journal = Journal::open_in_memory().expect("journal");
    let plan = with_plan(&journal);
    journal.mark_undone(plan).expect("first undo");
    assert!(matches!(
        journal.mark_undone(plan),
        Err(JournalError::PlanAlreadyUndone(_))
    ));
}

#[test]
fn an_unknown_plan_is_named_rather_than_silently_empty() {
    let journal = Journal::open_in_memory().expect("journal");
    assert!(matches!(
        journal.plan_by_id(PlanId(99)),
        Err(JournalError::UnknownPlan(99))
    ));
}

#[test]
fn a_plan_remembers_the_folder_and_the_snapshot_it_was_built_from() {
    // The folder because operations only know paths; the snapshot because a
    // saved plan.json applied later has to be refusable as stale.
    let journal = Journal::open_in_memory().expect("journal");
    let plan = journal
        .begin_plan("/Users/me/Downloads", "abc123")
        .expect("plan");
    let stored = journal.plan_by_id(plan).expect("read back");
    assert_eq!(stored.folder, "/Users/me/Downloads");
    assert_eq!(stored.snapshot, "abc123");
    assert!(stored.applied_at > 0);
    assert!(stored.undone_at.is_none());
}

#[test]
fn rmdir_survives_a_round_trip_through_the_journal() {
    // Added in v7 alongside MkDir, which had been defined since slice 2 and
    // written by nothing until the planner.
    let journal = Journal::open_in_memory().expect("journal");
    let op = journal
        .begin(&NewOp {
            kind: OpKind::RmDir,
            source: Some(Location::new("/root", "old")),
            destination: None,
            size: None,
            link: None,
            link_id: None,
        })
        .expect("op");
    journal
        .finish(op, &Outcome::Committed { hash: None })
        .expect("finish");
    let found = journal.recent(1).expect("recent");
    assert_eq!(found[0].kind, OpKind::RmDir);
}
