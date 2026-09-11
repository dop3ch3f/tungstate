# Slice 4b tour: the connection seam

Read this next to the diff. Nothing here changes what a drain *does* — the
guarantee is the same one slice 3 shipped. What changes is where a link end can
point.

Files: two new crates, `crates/tungstate-secret` and
`crates/tungstate-backend-opendal`. The journal gained `connections.rs` and
migration v4. The CLI gained `connection.rs` and `ends.rs`. The GUI changed
shape without changing behaviour.

The five Rust subjects, in the order they appear: **driving async code from
synchronous code**, **a factory returning `Box<dyn Trait>` and why it became
fallible**, **`Option<Id>` as a domain "none" and why `NULL` beat an enum
variant**, **walking `Path::components()` instead of joining**, and **a trait
with two implementations as the standard way to keep a test off the machine's
real state**.

---

## Part 1: Driving async from sync, without `block_on`

This is the load-bearing trick of the slice, and it is a genuine trap rather
than a style preference.

OpenDAL has been async-only since 0.54. It ships a convenience wrapper,
`opendal::blocking::Operator`, and tungstate deliberately does not use it. The
wrapper calls `tokio::runtime::Handle::block_on`, which **panics** if the thread
calling it is already inside a tokio runtime. The GUI's Tauri commands run on
exactly such a thread. That is not a test failure; it is a crash in front of the
user.

`Handle::spawn` has no such restriction. It is legal from any thread, inside a
runtime or not. So (`crates/tungstate-backend-opendal/src/runtime.rs`):

```rust
static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| { /* two worker threads */ });

pub(crate) fn dispatch<F>(future: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    RUNTIME.handle().spawn(async move {
        let _ = tx.send(future.await);
    });
    rx.recv().expect("…the runtime dropped a task without answering")
}
```

Four things to notice.

**`LazyLock<Runtime>`.** A `static` in Rust must be initialisable at compile
time, and building a thread pool is not. `LazyLock` (stable since 1.80) runs the
closure on first access and then hands out `&Runtime` forever. It is the modern
replacement for the `once_cell::sync::Lazy` you will see in older code.

**`F: Send + 'static` is `spawn`'s demand, not ours.** The runtime may move the
future to another thread and may run it after this function returns, so the
future cannot borrow anything from the caller's stack. That is why every method
in the adapter clones the `Operator` (cheap — it is an `Arc` inside) and turns
the path into an owned `String` *before* building the future. When you see

```rust
let operator = self.operator.clone();
let owned = self.key(path)?;
dispatch(async move { operator.stat(&owned).await })
```

the two `let`s are not ceremony. They are what makes `async move` able to take
ownership of everything it touches.

**The channel is a `sync_channel(1)`, not a oneshot.** We want a *blocking*
receive on the calling thread. `rx.recv()` parks the thread until the spawned
task sends, which is precisely the sync/async join we need.

**`.expect` rather than an invented error.** `recv` can only fail if the sender
was dropped without sending, which can only happen if the spawned task panicked.
A panic in the adapter is a bug in the adapter, and converting it into a
per-file transfer error would turn a bug into a silent mid-drain failure that
nobody investigates.

There is a test for the exact case the wrapper gets wrong:

```rust
#[test]
fn dispatching_from_inside_a_runtime_does_not_panic() {
    let outer = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let answer = outer.block_on(async { dispatch(async { 42 }) });
    assert_eq!(answer, 42);
}
```

This is DESIGN.md §7's "adapter at the boundary". The async blast radius is one
file. Nothing above it knows tokio exists.

### The same trick twice more, for `&mut` and for ownership

`Reader::read(&self, range)` takes `&self`, so the future only needs a shared
handle: `Arc<Reader>`, cloned into each spawn.

`Writer::write(&mut self, …)` takes `&mut self`, which cannot be shared. So the
writer is *moved into* the future and *moved back out* with the result:

```rust
let (writer, result) = dispatch(async move {
    let result = writer.write(bytes).await;
    (writer, result)
});
self.writer = Some(writer);
```

The `Option<Writer>` field exists solely to make that hand-off expressible:
`self.writer.take()` leaves `None` behind while the value is away. This is the
standard Rust move for "I need to temporarily give something away and get it
back", and you will reach for it again.

The payoff: the engine's 1 MiB streaming loop in `transfer/src/lib.rs` is
completely unchanged and still hashes in one pass. It has no idea whether it is
writing to a file or to a socket.

One guard in the `Read` impl is worth a sentence, because it is the kind of
thing `std::io::Read`'s contract invites you to get wrong. Returning `Ok(0)`
means end of file. If a remote stops sending partway through, returning `Ok(0)`
would tell the engine the file ended — and since the destination would then be
exactly as short as what we sent, `verify` would compare a short length against
a short length and *pass*. So the adapter, which is the only place that knows
the real length, turns "zero bytes before the end" into
`ErrorKind::UnexpectedEof` instead.

---

## Part 2: A factory returning `Box<dyn Backend>`, and why it had to be fallible

Slice 1 introduced `Box<dyn Trait>`. This is the first place it pays rent.

```rust
pub fn open(
    endpoint: &Endpoint,
    journal: &Journal,
    secrets: &dyn SecretStore,
) -> Result<Box<dyn Backend>>
```

Every caller — CLI preview, CLI run, GUI browse, GUI preview, GUI run — used to
write `LocalBackend::new(path)`. Five sites, each of them an unexamined
assumption that a link end is on this machine. Now they all call `open`, and the
assumption lives in exactly one `match`.

The signature change worth dwelling on is not `Box<dyn Backend>`. It is
`Result<…>`. `LocalBackend::new` is infallible by design: it does no I/O, so a
backend on a directory that does not exist yet is a perfectly good value that
fails later, at first use. That cannot hold for a remote. Reaching a NAS
involves a network, a login, and a keychain read, and all three can fail before
a single file is considered. Pretending otherwise would just defer the failure
to the first file, where the error message has a filename in it and no
explanation.

So the call sites changed shape:

```rust
// before
let source = LocalBackend::new(link.source_root.clone());
let destination = LocalBackend::new(link.destination_root.clone());

// after
let (source, destination) = match ends_of(&link, journal) {
    Ok(pair) => pair,
    Err(error) => return fail(error.as_ref()),
};
```

and then `source.as_ref()` where `&source` used to go, because `Transfer::new`
wants `&dyn Backend` and we are holding a `Box<dyn Backend>`. `as_ref()` on a
`Box<T>` gives `&T`; with `T = dyn Backend` that is exactly the `&dyn Backend`
the engine wants. No cost, no copy, just a reborrow.

---

## Part 3: `Option<ConnectionId>` as "local", and why `NULL` beat a `Local` variant

```rust
pub struct Endpoint {
    /// `None` is the local filesystem.
    pub connection: Option<ConnectionId>,
    /// Absolute for local; relative to the connection's root for a remote.
    pub path: PathBuf,
}
```

A Rust programmer's instinct here is an enum:

```rust
enum Place { Local, Connection(ConnectionId) }   // not what we did
```

and in pure Rust terms that is the better type — it names the two cases and
`None` is doing double duty. The reason we did not is the *database*, and this is
worth understanding because it recurs.

The `links` table already had four rows and `ops` had a few hundred. Migration
v4 had to leave them meaning what they already meant. Written as nullable
columns, that is free:

```sql
ALTER TABLE links ADD COLUMN source_connection INTEGER REFERENCES connections (id);
```

Every existing row gets `NULL`, which by definition already means "local". No
backfill, no `UPDATE`, nothing that can half-finish. A `Local` variant would
have needed a discriminant written into every existing row — an `UPDATE` over
the whole table, inside a migration, on a file that might be on a laptop whose
lid closes. Additive-only migrations are the rule; this is what following it
costs and buys.

`Option<T>` in Rust is a real enum with a niche optimisation, so
`Option<ConnectionId>` is the same size as `ConnectionId` — eight bytes, no tag.
You pay nothing at runtime for the `Option`, and `None` reads naturally as "no
connection, so: here".

### The foreign key doing work Rust cannot

```sql
source_connection INTEGER REFERENCES connections (id)
```

with `PRAGMA foreign_keys = true` (already on since slice 2) means SQLite itself
refuses to delete a connection a link still points at. There is no check in
`delete_connection` doing this; the error comes back from the database and we
translate it:

```rust
rusqlite::Error::SqliteFailure(inner, _)
    if inner.code == rusqlite::ErrorCode::ConstraintViolation =>
        JournalError::ConnectionInUse(name.to_string()),
```

That is the right place for the rule. A Rust-side check protects the callers who
remember to call it; a foreign key protects everyone, including a future daemon
and `sqlite3` on the command line.

### One reversal, and it is the contentious one

`migrate` used to skip silently when the file's `user_version` was higher than
anything it knew, so an old binary could still read a newer journal. It now
returns `JournalError::TooNew`.

The reason the old behaviour was safe is that every stored link end was a local
absolute path, and an old binary reading `/Volumes/nas/inbox` got the right
answer even if it did not understand the columns beside it. That stops being
true here. A v4 end can be `inbox`, relative to a connection an old binary has
no table for — and an old binary would read `inbox` as a local relative path and
drain into it. Refusing to open is the only honest response. Argue with this one
if you disagree; it is the decision in the slice most worth a second opinion.

### Two queries that were wrong, not merely incomplete

`history` and `whereis` reconstructed a full path in SQL:

```sql
OR (src_root || '/' || src_path) = ?1
```

For a remote row that splices a connection-relative root onto a
connection-relative path and can collide with a genuine local file. (It was also
already questionable on Windows, where the separator is not `/`.) Both branches
are now scoped:

```sql
OR (src_connection IS NULL AND (src_root || '/' || src_path) = ?1)
```

There is a test with a local row and a remote row spelled identically, asserting
only the local one answers to the full path.

---

## Part 4: Walking `Components`, because `Path::join` is wrong for a remote key

`crates/tungstate-backend-opendal/src/keys.rs` is thirty lines and prevents a
whole category of corruption.

```rust
pub fn remote_key(path: &Path) -> Result<String> {
    let mut key = String::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if !key.is_empty() { key.push('/'); }
                key.push_str(&part.to_string_lossy());
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) =>
                return Err(BackendError::PathNotRelative(path.to_path_buf())),
            Component::ParentDir =>
                return Err(BackendError::PathEscapesRoot(path.to_path_buf())),
        }
    }
    Ok(key)
}
```

`PathBuf::from("2024").join("holiday.mp4")` is `2024/holiday.mp4` on Unix and
`2024\holiday.mp4` on Windows. Both are correct *as paths*. But a protocol key
is not a path: FTP and S3 take a string, `\` is an ordinary character in a
filename, and `2024\holiday.mp4` is a single file with a backslash in its name —
a different, corrupt key. On some servers it is worse than corrupt.

Three details:

**Matching `Component` exhaustively, not calling `is_absolute()`.** This is the
same reasoning as `local.rs` and it was found by Windows CI in slice 1:
`/etc/passwd` reports `is_absolute() == false` on Windows because it has no
drive prefix, yet it still escapes. Components catch both platforms with one
rule. Exhaustive matching also means a future `Component` variant becomes a
compile error here rather than a silent hole.

**Rejecting rather than normalising `..`.** We could resolve `a/../b` to `b`.
We do not, because the same reasoning as slice 1 applies: a path we did not
construct containing `..` is a bug or an attack, and silently fixing it hides
both.

**The proptest asserts the property, not the error variant.**

```rust
prop_assert!(!key.contains('\\'));
prop_assert!(!key.starts_with('/'));
prop_assert!(key.is_empty()
    || key.split('/').all(|p| p != ".." && p != "." && !p.is_empty()));
```

Note the last one checks *components*, not substrings. `a..b` is a perfectly
legal filename; a component that *is* `..` is not. Getting that distinction
wrong is how a property test becomes a false alarm nobody trusts.

The generator deliberately contains no backslash. A backslash inside a filename
on Unix is part of the name and passing it through verbatim is correct; what
must never happen is a backslash appearing because *we* joined with one.

---

## Part 5: A trait with two implementations, to keep tests off the machine

`crates/tungstate-secret` is three methods and two structs, and it exists mostly
so that tests are possible.

```rust
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, secret: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}
```

`KeyringStore` is the real thing: macOS Keychain, Windows Credential Manager,
Secret Service on Linux. `MemoryStore` is a `Mutex<BTreeMap>`.

Why not just use `KeyringStore` in tests? Because **headless Linux CI has no
D-Bus session and therefore no Secret Service.** A test against the real
keychain would fail on one of the three platforms tungstate promises, and on a
developer's Mac it pops a permission dialog mid-`cargo test`. The real path gets
one `#[ignore]`d round-trip test you run by hand on a desktop:

```rust
#[test]
#[ignore = "touches the machine's real keychain"]
fn a_keyring_store_round_trips_a_secret() { … }
```

This is the standard shape and you will use it constantly: a trait at the
boundary to the outside world, the real implementation for production, a fake
for tests. `Backend` itself is the same pattern one level up — which is why the
transfer suite could grow `CorruptingBackend` and `VanishingBackend` without the
engine knowing.

Note `Send + Sync` on the trait. `Sync` means `&dyn SecretStore` can be shared
across threads, which the factory needs because the GUI opens backends from a
worker thread. There is a test that does exactly that, so the bound is exercised
rather than merely declared.

### Where the secret is not

There is no `password` column in the `connections` table and no `secret_key`
column either. That is the point:

> **No secret ever enters this database.** The absence is structural, not a
> convention someone has to remember.

A password cannot leak into a `sqlite3` dump, a backup, or a bug report, because
there is nowhere to put one. On the CLI side the same reasoning gives you
`rpassword::prompt_password` and `--secret-stdin` but no `--password` flag: an
argument lands in shell history and is visible in `ps` to every other user on
the machine.

---

## Part 6: Two things the build found that the plan did not

Both are worth reading because both are the kind of bug that passes every test
you thought to write.

### The connection is the rail, not the link end

The first hand-run of the slice's own runnable outcome failed:

```
$ tungstate link preview viaopendal
error: `/tmp/tungstate-dest/inbox` is not reachable, or is not a directory
```

`inbox` had never been created. The adapter had been rooting its operator at the
*link end*, which made `root_token()` mean "that exact subfolder exists". Too
strict: a destination folder nobody has made yet is an ordinary first write, the
same as `2024/` inside it.

The fix moves the rail up one level. The operator is rooted at the **connection**
and the link end is carried as a key prefix that every call joins on. So:

- `root_token()` asks "is the connection there?" — the storage, the thing that
  can unmount.
- `create_dir_all("")` on first write makes `inbox`.
- A preview still creates nothing, because it only stats.

The integration test had pre-created `inbox` and so had happily agreed with the
wrong model. Hand-running the documented sequence is what caught it, which is
why step 2 of every slice's verification is "do it by hand".

### `Operator::stat("/")` does not touch the store

Then the replacement rail did not work either, and this one was nastier.

```rust
// looks right, is not
let meta = dispatch(async move { operator.stat("/").await })?;
```

OpenDAL short-circuits this. In `layers/simulate.rs`:

```rust
if path == "/" {
    return Ok(RpStat::new(MetadataBuilder::dir().build()));
}
```

It answers from thin air without asking the store anything. So `root_token()`
would have returned `Ok` forever — including after the NAS unmounted — and the
drain would have kept writing to whatever was at that path and kept deleting
originals. That is the single failure mode tungstate exists to prevent, and it
would have been silently disabled by a one-line call that reads perfectly.

The fix is an explicit `Anchor`:

```rust
pub enum Anchor {
    /// A directory on this machine. One `stat` syscall, exactly as
    /// `LocalBackend` does it, and exactly as cheap.
    LocalDir(PathBuf),
    /// Somewhere only the protocol can reach. Listing the root is the cheapest
    /// thing OpenDAL offers that genuinely goes to the store.
    Store,
}
```

The `fs` scheme gets `LocalDir`, so reachability costs exactly one syscall per
file, the same as slice 3. `Store` is honest but once-per-file listing is
quadratic against a large root, and it is flagged in the source for slice 4c to
fix when FTP makes it a real cost.

The regression test names the trap in its comment, so a future refactor back to
`stat("/")` fails loudly:

```rust
fn a_connection_that_changes_underneath_us_stops_the_drain() { … }
```

**The general lesson.** A dependency's convenience layer can be *correct* for
its own purposes and *wrong* for yours. OpenDAL synthesising a root is sensible
for an S3 bucket, which has no root object to stat. It is catastrophic for a
tool whose entire value proposition is noticing that a disk went away. When a
library answers a question suspiciously cheaply, go and read how.

---

## What to look at in the diff

In rough order of how much thought is in them:

1. `crates/tungstate-backend-opendal/src/runtime.rs` — 60 lines, the whole
   async bridge.
2. `crates/tungstate-backend-opendal/src/backend.rs` — `Anchor`, `root_token`,
   and the read/write adapters.
3. `crates/tungstate-journal/src/schema.rs` — migration v4 and the `TooNew`
   reversal.
4. `crates/tungstate-journal/src/lib.rs` — the two scoped queries.
5. `crates/tungstate-cli/src/ends.rs` — the two-character rule, and the tests
   that pin `C:\Users\x`.
6. `crates/tungstate-transfer/src/tests.rs` — the bottom of the file, where the
   whole suite runs a second time over the seam.
