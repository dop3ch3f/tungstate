# Slice 2: The provenance journal

**Goal:** an append-only record of every operation tungstate performs, durable enough that a crash mid-drain is recoverable and complete enough to answer "where did this file go?".

**Runnable outcome:** `tungstate log <path>` and `tungstate whereis <path>` print real history from a real database.

**This brief is the spec.** It states what gets built and why. The walkthrough of the shipped code is in [02-tour.md](02-tour.md).

---

## Why this comes before the drain

Slice 3 copies a file to the NAS, verifies it, then deletes the source. That sequence has to survive the lid closing between any two steps. The only way to make it survive is to write down what you are about to do *before* you do it, so that on restart you can tell the difference between "not started", "half done" and "finished".

That record is this slice. It also happens to answer the question the user actually cares about day to day, which is where a given file ended up.

## Decisions

**One journal per machine**, at the platform state directory (`~/.local/share/tungstate/journal.db` on Linux, the equivalent elsewhere via the `directories` crate). A drain moves a file from folder A to folder B; that is one story and it should live in one place. Per-folder journals would split it across two databases and make `whereis` open every database on the machine to answer one question.

**Keep every entry.** A million rows is a few hundred megabytes and SQLite is untroubled by it. `whereis` stays truthful forever, which is the whole point of it. Revisit with real growth data rather than guessing now.

**Write-ahead discipline.** Every operation is recorded as `Intended` before it touches the filesystem, then updated to `Committed`, `Failed` or `Skipped` afterwards. A row still saying `Intended` at startup means the process died mid-operation, and that is exactly the set slice 3 needs in order to resume.

**SQLite in WAL mode with `synchronous = NORMAL`.** WAL lets readers run while a write is in flight. `NORMAL` skips an `fsync` on every commit, which matters when a drain commits tens of thousands of times.

That is a durability trade, so here is the reasoning explicitly. With `NORMAL`, a power cut can lose the last few journal writes. The dangerous version of that would be losing a write that says "file safely copied" *after* the source was deleted. Slice 3 avoids it by ordering: the destination file is fsynced, then the journal records the commit, then the source is removed. If the journal write is lost, the worst outcome is that tungstate re-copies a file it had already copied. Redundant work, never lost data. `FULL` would buy nothing against that ordering and would cost an fsync per file.

**Migrations via `user_version`.** SQLite carries an integer in the file header for exactly this. A small runner applies each numbered step in order and records where it got to. No migration framework for a schema this size.

**`Mutex<Connection>`.** `rusqlite::Connection` is `Send` but not `Sync`, and slice 4 runs transfers on several threads. A mutex makes `Journal` shareable, and costs nothing real because SQLite serialises writers anyway.

## Shape

New crate `crates/tungstate-journal`.

- `JournalError` via `thiserror`, wrapping `rusqlite::Error` with context about what was being attempted.
- `OpId(i64)` newtype, as with `FolderId`.
- `OpKind`: `Copy`, `Move`, `Rename`, `Remove`, `MkDir`. Stored as text so a database dump is readable.
- `OpStatus`: `Intended`, `Committed`, `Failed`, `Skipped`.
- `NewOp`, the intent record: kind, source and destination locations, size, link name, plan id.
- `Op`, a row read back, adding id, timestamps, status, hash, error and any metadata that could not be preserved.
- `Journal::open(path)`, `Journal::open_in_memory()` for tests.
- `begin(&NewOp) -> OpId` writes the intent.
- `finish(OpId, Outcome)` records the result.
- `incomplete() -> Vec<Op>` is crash recovery: everything still `Intended`.
- `history(&Path) -> Vec<Op>` is `tungstate log`.
- `whereis(&Locator) -> Vec<Op>` accepts a path or a content hash.

Indexes on source path, destination path, hash, and timestamp, because those are the four questions anyone asks.

CLI: `tungstate log <path>` and `tungstate whereis <path>` become real commands, replacing their stubs.

## Tests

- Round trip: begin, finish, read back, every field intact.
- **Crash recovery:** begin an operation, drop the `Journal` without finishing, reopen the same file, and assert the operation comes back from `incomplete()`. This is the test the drain's correctness rests on.
- Migrations run on a fresh database and are idempotent when run again on an existing one.
- `history` returns operations oldest first and only for the path asked about.
- `whereis` finds a file by its destination path after a move, and by hash.
- Concurrency: several threads writing at once all succeed and every row is present, proving the mutex and WAL configuration hold.
- Every test uses its own temp file or an in-memory database, so the suite stays parallel-safe.

## Acceptance criteria

- [ ] `cargo build --locked` clean, `cargo test --locked` green
- [ ] `cargo clippy --all-targets --locked -- -D warnings` silent, real `# Errors` docs
- [ ] `cargo fmt --all --check` silent
- [ ] CI green on Linux, macOS and Windows
- [ ] `tungstate log` and `tungstate whereis` return real rows
- [ ] A crash-recovery test that fails if write-ahead ordering is broken
- [ ] No `unwrap()` in `src/`

## Out of scope

No hashing; the hash column stays null until slice 3 fills it. No `blame`, no undo, no retention or compaction. No cross-machine queries. Nothing writes to the journal automatically yet, because nothing performs file operations until slice 3.
