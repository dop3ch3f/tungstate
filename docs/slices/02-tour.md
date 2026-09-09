# Slice 2 tour: the provenance journal

Read this next to the diff. Three files: `crates/tungstate-journal/src/lib.rs` is the public shape, `schema.rs` owns the tables and migrations, `tests.rs` holds the suite. The CLI grew two real commands.

The journal has two jobs, and the second is the one that matters. It answers "where did this file go?", and it is what makes slice 3's drain survive the lid closing mid-transfer.

---

## Part 1: Why write it down *before* doing it

This is the whole idea of the slice, so it is worth being precise.

Slice 3 will copy a file to the NAS, verify it, and delete the source. If the process dies between any two of those steps, what does the next run see? Without a record, only a source file and a destination file, with no way to tell "this copy finished and was verified" from "this copy was halfway through when the power went".

So every operation is written to the database as `Intended` *before* the filesystem is touched, and updated to `Committed`, `Failed` or `Skipped` afterwards. That gives a precise definition of interrupted work: any row still saying `Intended` at startup. `Journal::incomplete()` returns exactly that set, and it is the first thing the drain will call on startup.

```rust
let id = journal.begin(&op)?;   // written down first
// ... do the actual work ...
journal.finish(id, &Outcome::Committed { hash })?;
```

The test that guards this is `an_interrupted_operation_survives_a_restart`. It begins an operation, drops the `Journal` without finishing it, which is what a crash looks like from the database's point of view, reopens the same file, and asserts the operation comes back. If someone later "optimises" by writing the row after the work instead of before, that test fails. This is the difference between a test that checks a function and a test that locks a property.

---

## Part 2: Modules, and what `mod` actually does

```rust
mod schema;
```

New in this slice. A module is a namespace inside a crate, and `mod schema;` tells the compiler to look for `schema.rs` beside this file and pull it in.

Note there is no `pub`. `schema` is private, so nothing outside this crate can call `schema::prepare`. Rust defaults everything to private and you opt into visibility, which is the opposite of most languages. Inside the module, `pub(crate) fn prepare` means "public to this crate, invisible outside it". That fine-grained control is why the crate can expose a small, deliberate surface while splitting the implementation across files.

`#[cfg(test)] mod tests;` at the bottom does the same thing for the test file, but only when compiling tests. The test binary never ships.

---

## Part 3: Enums that cross a database boundary

```rust
impl OpKind {
    fn as_str(self) -> &'static str { ... }
    fn parse(raw: &str) -> Option<Self> { ... }
}
```

`OpKind` and `OpStatus` are stored as text, not integers. That costs a few bytes per row and buys something worth more: `sqlite3 journal.db "select * from ops"` is readable by a human debugging a stuck drain at midnight. Integer enums mean cross-referencing a table against source code at the worst possible moment.

**`&'static str` is your first lifetime.** It means "a string slice that lives for the whole program". String literals in the source are baked into the binary, so they qualify. You will meet other lifetimes later; this one is the simple case, and it is why `as_str` can return a borrowed string without allocating.

**`parse` returns `Option<Self>`, and the fallback is deliberate:**

```rust
kind: OpKind::parse(&kind_raw).unwrap_or(OpKind::Copy),
```

A row whose kind this binary does not recognise came from a *newer* version of tungstate that added a variant. Falling back lets an older binary still read the journal instead of refusing to start. The alternative, returning an error, would mean downgrading tungstate bricks your history. This is the kind of forward-compatibility decision that is nearly free now and impossible to retrofit.

---

## Part 4: The type that will not fit

```rust
fn size_to_sql(size: u64) -> i64 {
    i64::try_from(size).unwrap_or(i64::MAX)
}
```

The compiler rejected the first version of this code, and the error was informative: `the trait bound u64: ToSql is not satisfied`.

SQLite has exactly one integer type, a signed 64-bit `INTEGER`. There is no unsigned. File sizes are naturally unsigned, so the two models genuinely disagree.

There were two ways out. Change the public API to `Option<i64>`, which leaks a storage detail into the domain model and means every caller thinks about negative file sizes forever. Or keep `u64` in the API, which is what a size *is*, and convert at the boundary. The second is right: storage concerns belong at the storage layer.

Reading back is more interesting than writing:

```rust
size: row.get::<_, Option<i64>>("size")?.and_then(|v| u64::try_from(v).ok()),
```

A negative value in that column can only mean corruption. `and_then` with `try_from(...).ok()` turns it into `None`, meaning "size unknown", rather than `0`, which would read as a genuine empty file. Choosing the honest failure over the convenient one matters most in code that decides whether to delete an original.

---

## Part 5: Migrations, using a field SQLite already has

```rust
const MIGRATIONS: &[&str] = &[ /* v1: create the ops table and its indexes */ ];
```

Every SQLite file has a `user_version` integer in its header, unused by SQLite itself and provided for exactly this. The runner reads it, skips migrations already applied, and runs the rest in order.

```rust
let tx = conn.unchecked_transaction()?;
tx.execute_batch(migration)?;
tx.pragma_update(None, "user_version", index + 1)?;
tx.commit()?;
```

**The transaction is the point.** The migration and its version bump commit together or not at all. Without it, a crash between the two leaves a database with the new tables but the old version number, so the next start tries to create tables that already exist and fails permanently. That is a genuinely nasty bug class, and one transaction eliminates it.

`opening_an_existing_journal_does_not_re_run_migrations` locks this. If the version gate were removed, it fails immediately with "table ops already exists".

No migration framework. Migration frameworks earn their keep on schemas with dozens of tables and multiple developers; here it would be more code than the thing it manages.

---

## Part 6: The pragmas, and a durability decision worth understanding

```rust
let _ = conn.pragma_update(None, "journal_mode", "WAL");
conn.pragma_update(None, "synchronous", "NORMAL")?;
```

**WAL**, write-ahead logging, lets readers run while a write is in flight and survives a crash mid-transaction. The `let _ =` is deliberate: an in-memory database has no WAL and refuses, which is not an error worth propagating.

**`synchronous = NORMAL`** is the one to understand, because it is a data-safety trade in a tool that deletes your files.

`FULL` fsyncs on every commit. A drain of a full laptop commits tens of thousands of times, and an fsync per commit is a real cost on a spinning disk. `NORMAL` only fsyncs at checkpoints, so a power cut can lose the last few journal writes.

The question is what a lost write actually costs. Slice 3 will order its work like this:

1. Write the destination file, fsync it.
2. Journal the commit.
3. Remove the source.

If step 2's write is lost to a power cut, the next run sees an operation still marked `Intended` and copies the file again. The destination already has it, so the result is a redundant transfer. Wasted minutes, never lost data. The dangerous ordering, deleting the source before the journal is durable, is a bug we do not write rather than a setting we compensate for.

That reasoning is in the code as a comment, because "why NORMAL and not FULL" is exactly what a reviewer should challenge.

---

## Part 7: `Mutex<Connection>`, and locks that outlive a panic

```rust
pub struct Journal {
    conn: Mutex<Connection>,
}
```

`rusqlite::Connection` is `Send`, meaning it can move between threads, but not `Sync`, meaning it cannot be *shared* between them. Slice 4 runs transfers on several threads, and they all need to journal. Wrapping in a `Mutex` makes `Journal` shareable, and costs nothing real because SQLite serialises writers regardless.

The `concurrent_writers_all_land` test runs eight threads writing two hundred operations and asserts every one is present. It deliberately does not assert ordering between threads, because ordering is not part of the contract, and a test that asserts it would be flaky for no reason.

**The lock helper is the interesting line:**

```rust
fn lock(&self) -> MutexGuard<'_, Connection> {
    self.conn.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
```

Rust mutexes are *poisoned* when a thread panics while holding one. Every later `lock()` returns an error, on the theory that the protected data might be inconsistent.

Here that default is wrong. A panic mid-write leaves the SQLite connection perfectly sound; SQLite's own transactions handle the consistency. Propagating poison would turn one panic in one transfer into a permanently dead journal for the whole process, which is a much worse outcome than the thing it protects against. So we recover the guard and carry on. The comment in the code says why, because silently ignoring a poisoned lock is otherwise exactly the sort of thing a reviewer should flag.

---

## Part 8: The CLI, and printing an error properly

```rust
fn fail(error: &dyn std::error::Error) -> ExitCode {
    eprintln!("error: {error}");
    let mut source = error.source();
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
    ExitCode::FAILURE
}
```

Remember `#[source]` on the error types from slice 1. This is what it was for. Printing only the top-level message would give you `journal query failed: locating by path` and nothing about *why*. Walking the chain gives you the SQLite error underneath. Ten lines, and it is the difference between a debuggable tool and a frustrating one.

`&dyn std::error::Error` is the same dynamic dispatch idea as `Box<dyn Backend>`, without the box: any error type at all, behind a reference.

**One small piece of judgement in `whereis`:**

```rust
fn is_hash(target: &str) -> bool {
    target.len() == 64 && target.chars().all(|c| c.is_ascii_hexdigit())
}
```

`whereis` accepts either a path or a hash, and rather than making you say which, it looks at the shape. A BLAKE3 digest is 64 hex characters and no real path looks like that. The test for the boundary case is manual: a 64-character non-hex string is treated as a path, correctly.

---

## Part 9: The snapshot test earning its keep, again

Adding `log` and `whereis` broke `help_output_is_stable`, exactly as designed. The diff showed two added lines and clap re-aligning its column, nothing else, so the change was safe to accept.

That is the value: not that the help text is frozen, but that a change to the command surface is impossible to make *accidentally*. If adding a command had also renamed one, the same diff would have shown it.

---

## Verified end to end

Not just unit tests. The real binary was run against a real database:

```
$ tungstate log /Users/dop3ch3f/Videos/holiday.mp4
   1 ok          /Users/…/holiday.mp4 -> /Volumes/nas/inbox/2026/09/holiday.mp4
$ tungstate whereis aaaa…  (64 hex chars)
   1 ok          /Users/…/holiday.mp4 -> /Volumes/nas/inbox/2026/09/holiday.mp4
```

The database landed at `~/Library/Application Support/tungstate/journal.db`, reported `user_version = 1`, `journal_mode = wal`, `synchronous = 2` (NORMAL), which is exactly what the code asks for. The test row was removed afterwards.

Worth noting one thing that went wrong while doing that, because it is a good lesson about trusting a query: seeding the row by hand with `'a'*64` in SQL stored `0`, not a 64-character string, because SQLite coerced `'a'` to a number and multiplied. `whereis` by hash then correctly found nothing, and for a moment that looked like a bug in the code. It was a bug in the test data. When a query returns nothing, check what is actually in the column before changing the query.

---

## What to look at in review

- Is `Intended` genuinely written before any filesystem work happens? Slice 3 depends on it absolutely.
- Is `synchronous = NORMAL` the right trade, given the ordering argued for above? This is the decision most worth challenging.
- Does recovering from a poisoned mutex sit right with you, or would you rather the journal fail loudly?
- Is `history` matching the correct set? It matches a path at either end so a file's history survives a move, which also means a move shows up in the history of both its old and new names.
