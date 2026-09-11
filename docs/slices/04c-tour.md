# Slice 4c tour: FTP

Read this next to the diff. Slice 4b built the seam; this is the first real
protocol going through it, and the protocol pushed back harder than the
roadmap expected.

Files: `tungstate-transfer/src/lib.rs` gained a commit path that does not
rename, and a sweep. `tungstate-backend-opendal` gained the FTP service, a
streaming reader, and a cheaper reachability check.
`tungstate-cli/tests/ftp.rs` is new and runs against a real server.

The subjects, in the order they appear: **a capability flag that finally does
something**, **bridging a `Stream` to `io::Read`**, **reading your
dependency's source instead of its README**, and **why a sweep needs a narrow
pattern**.

---

## Part 0: What reading the dependency found

This slice began by reading `opendal-service-ftp` 0.59.1 rather than by
writing code, and that hour changed the plan four times. Worth recording as a
habit, not just as facts.

**`rename` and `copy` both return `Unsupported`.** Not because FTP lacks them —
the protocol has `RNFR`/`RNTO`, and `suppaftp`, which OpenDAL uses underneath,
exposes `rename()` on both its clients. OpenDAL simply did not wire it up. So
tungstate's commit step, *stream to a temp name → verify → rename into place*,
cannot run at all over FTP.

**But the writer is internally atomic anyway.** `FtpLazyWriter` streams to
`<basename>.<8 random chars>` and issues `RNFR`/`RNTO` to the real name on
`close()`. So writing straight to the final name still publishes atomically.
This is the single fact that made the degradation small.

**Every ranged read is a new data connection.** `FtpReadStream::new` sends
`REST <offset>` then `RETR`. Slice 4b's reader issued one ranged read per 1 MiB
chunk, which for a 4 GB file is four thousand FTP data connections.

**A crash leaks a file nothing can find.** That random temp name is generated
from a fresh `RandomState` each time. Neither OpenDAL nor we can reconstruct
it, so an interrupted transfer leaves `holiday.mp4.k3xq9wpz` on the NAS
forever.

None of that is in any documentation. The lesson is the one from slice 4b's
`stat("/")` trap, repeated: **when a dependency sits between you and something
you have promised to be careful about, read its source.**

---

## Part 1: A capability flag that finally does something

`Capabilities::atomic_rename` has existed since slice 1 and nothing read it.
A flag defined and never consulted is a small lie: it reads like a guarantee
the code honours, and it is not.

```rust
if !self.destination.capabilities().atomic_rename {
    return self.write_in_place(file, destination);
}

let temp = temp_name(destination, op);
let (hash, written) = self.stream(&file.path, &temp)?;
// … verify, then rename into place
```

`write_in_place` is the whole of the other branch:

```rust
fn write_in_place(&self, file: &walk::File, destination: &Path) -> Result<String> {
    let (hash, written) = self.stream(&file.path, destination)?;
    if let Err(error) = self.verify(file, destination, &hash, written) {
        let _ = self.destination.remove_file(destination);
        return Err(error);
    }
    Ok(hash)
}
```

There is no third option. Without rename, the only route to the final name is
to write to it.

What still protects the commit is `WriteFinish::finish`, which the trait has
always defined as "commit everything written durably, then close". *How* a
backend achieves that is its business — a local file fsyncs, OpenDAL's FTP
service renames its own temp into place. The engine does not need to know, and
deliberately does not ask.

**Be honest about what is lost.** A rename backend can only ever leave a
`.part`; here the real name may hold a partial, so another program watching the
folder could see an incomplete file. No file is lost either way, because the
source is untouched until the journal commit that never happened.

How wide that window really is turns out to depend on the backend, and the
engine cannot tell. Killing a 3 GB drain over FTP mid-file left the destination
directory *completely empty*: OpenDAL had been writing to its own temp so the
real name never existed, and vsftpd deletes failed uploads. A different server,
or a different service, will not be so tidy. So the engine assumes the worst —
recovery deletes the real name and sweeps — and is simply right in the cases
where there was nothing to clean up.

### Recovery has to follow the same branch

```rust
let atomic = self.destination.capabilities().atomic_rename;
...
if atomic {
    let _ = self.destination.remove_file(&temp_name(&destination.path, op.id));
} else {
    let _ = self.destination.remove_file(&destination.path);
    self.sweep_orphans(&destination.path);
}
```

Deleting a file under its real name during recovery deserves a second look, and
it is safe for a specific reason worth stating: `copy_to` applies the source
policy only *after* the journal commit, and this operation is in the journal as
`intended`, which means it never got there. The source is still on disk. That
is the same asymmetry slice 3 was built around — the worst case is a redundant
copy, never a lost file — reused in a new place.

### One conflict action cannot survive this

`ConflictAction::Replace` moves the existing file into quarantine and then
lands the incoming one. Moving needs rename. Doing only the second half would
destroy the very file Replace promises to keep.

```rust
if !self.destination.capabilities().atomic_rename {
    return Err(TransferError::ReplaceNeedsRename { path: file.path.clone() });
}
```

Refusing is the only honest answer, and the CLI refuses earlier still — at
`link add`, rather than halfway through a drain. `Quarantine` and `Rename` both
keep working, because neither has to move anything that is already there.

---

## Part 2: Bridging a `Stream` to `io::Read`

The engine reads through `std::io::Read` and OpenDAL produces a
`futures::Stream` of buffers. Slice 4b dodged the mismatch with ranged reads;
FTP makes that cost four thousand connections, so the bridge has to be built
properly.

```rust
struct OpendalRead {
    /// `None` only while the stream is away inside a spawned future.
    stream: Option<BufferStream>,
    pending: Buffer,
    finished: bool,
}
```

Three fields, and each is there for a reason.

**`pending`** exists because the two sides disagree about size. The service
hands out buffers of whatever length it feels like; the engine asks for 1 MiB.
Neither can dictate to the other, so something has to hold the remainder.

**`Option<BufferStream>`** is the same `take()`-and-put-back move the writer
uses. `Stream::next` needs `&mut self`, and `spawn` needs an owned `'static`
future, so the stream is moved *into* the future and returned *with* the item:

```rust
let (stream, item) = dispatch(async move {
    let item = StreamExt::next(&mut stream).await;
    (stream, item)
});
self.stream = Some(stream);
```

If you saw this in slice 4b's writer and thought it was a one-off workaround —
it is not. It is the standard Rust idiom for "lend something out and get it
back", and you will keep meeting it.

**`finished`** distinguishes two things that look alike. A service may hand
back an *empty* buffer without meaning end of stream, so the read loop keeps
pulling until there is either data or a genuine `None`:

```rust
while self.pending.is_empty() {
    if self.finished { return Ok(0); }
    self.pull()?;
}
```

Getting that wrong would return `Ok(0)` early, which in `io::Read` means end of
file. The engine would write a short file, and because the destination would
then be exactly as short as what was sent, `verify` would compare a short
length against a short length and **pass**. A truncated file, verified, and the
original deleted. The two-line loop is load-bearing.

Handing out the data uses `Buffer::split_to`, which advances the rope without
copying the tail:

```rust
let head = self.pending.split_to(take);
buf[..take].copy_from_slice(&head.to_vec());
```

A 4 MiB buffer read out in 1 MiB pieces costs four copies, not four copies plus
three shuffles of the remainder.

---

## Part 3: Reachability, and not being quadratic about it

Slice 4b left a note in the source saying `Anchor::Store` listed the connection
root once per file, which is fine for a small root and quadratic for a large
one. FTP makes that real, so:

```rust
Anchor::Store => {
    if !self.prefix.is_empty()
        && self.stat_key(&format!("{}/", self.prefix), Path::new("")).is_ok()
    {
        return Ok(RootToken { device: None });
    }
    // Either this link end is the connection root, or its folder has not been
    // made yet. Listing is the only thing left that distinguishes "not
    // created" from "connection is gone".
    dispatch(async move { operator.list("/").await }).map_err(|_| unreachable())?;
}
```

One `stat` in the steady state. The listing only runs while the destination
folder does not yet exist, which is at most once per run instead of once per
file.

Note what this is *not*: a cache. The check still goes to the server before
every file, because the whole point is noticing that the NAS went away between
one file and the next. It is the same check, done cheaply.

---

## Part 4: A sweep needs a pattern narrow enough to trust

Deleting files the user did not ask you to delete is the worst thing a tool
like this can do. The sweep exists because OpenDAL leaves unreachable litter,
but that justification does not extend one character further than it has to.

```rust
const TEMP_SUFFIX_LENGTH: usize = 8;

fn is_orphan_temp(candidate: &str, name: &str) -> bool {
    let Some(suffix) = candidate
        .strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('.'))
    else {
        return false;
    };
    suffix.len() == TEMP_SUFFIX_LENGTH && suffix.chars().all(|c| c.is_ascii_alphanumeric())
}
```

Four conditions, all required: the candidate starts with *this* file's exact
name, then a literal dot, then exactly eight characters, all alphanumeric. The
test enumerates what must survive, which is the more interesting half:

```rust
"a.mp4"            // the file itself
"a.mp4.txt"        // three characters
"a.mp4.backup"     // six
"a.mp4.k3xq9wpzz"  // nine
"a.mp4.k3xq9wp-"   // not alphanumeric
"a.mp4k3xq9wpz"    // no dot
"b.mp4.k3xq9wpz"   // a different file's temp
```

And the sweep is scoped twice over: only the destination directory of an
operation *this link* left interrupted, and only when the backend cannot
rename. A file that merely looks like a temp, next to a file nobody was
transferring, is never even looked at.

Worth saying plainly: against vsftpd the sweep had nothing to do, because that
server deletes failed uploads itself. It exists for the servers that do not,
and the integration test stages a real orphan rather than hoping one appears.

The assumption it does rest on — that no second process is mid-write to the
same destination path — is stated in the comment rather than left implicit. A
link runs one at a time today. When slice 4 adds concurrency, that comment is
the thing that will need re-reading.

---

## Part 5: What the integration tests are for, and what they found

Six tests in `crates/tungstate-cli/tests/ftp.rs`, behind
`--features ftp-integration`, driving the real binary against a real vsftpd in
Docker. Linux CI only.

They found three things that unit tests with a fake backend could not have.

**The secret store seam from slice 4b was broken.** `TUNGSTATE_SECRETS=memory`
builds a fresh store *per process*, so what `connection add` stored was gone by
the time `connection test` ran. The fix is a real feature rather than a test
hack: `EnvOverride` reads `TUNGSTATE_SECRET_<NAME>` before asking the keychain.
That is how a NAS daemon or a container will have to be fed a password anyway
(DESIGN.md §6b), and it is how the tests feed one now — the parent sets the
variable on each child.

It also happens to be the only way to test `EnvOverride` at all:
`std::env::set_var` is `unsafe`, the workspace *forbids* `unsafe`, and `forbid`
cannot be lifted by an `allow`. A cross-process test was the honest answer, not
a workaround.

**`--root /` does not mean what it looks like.** OpenDAL issues a `CWD` to the
connection root, so `/` is the server's real root, not the directory the login
lands in. On the test server the account cannot write there, and the failure
surfaced as `553 Could not create file` from a `MKD` that OpenDAL had silently
swallowed. `connection test` now prints the root it checked, so
"reachable, 18 entries" can no longer be reassuring while pointing at the
server's `/etc`.

**A per-file failure report that does not say why is not a report.** Chasing
the 553 meant the end-of-run summary was printing "transfer of `a.txt` failed"
and nothing else, because `Failure::reason` was `error.to_string()` with the
cause chain thrown away. It now walks `source()`, like the CLI's top-level
error printer already did. That is a fix for every backend, not just FTP; it
was simply never painful enough to notice before.

---

## What to look at in the diff

1. `tungstate-transfer/src/lib.rs` — `write_in_place`, the recovery branch,
   `is_orphan_temp`, and `explain`.
2. `tungstate-backend-opendal/src/backend.rs` — `OpendalRead`, and the
   `Anchor::Store` arm of `root_token`.
3. `tungstate-backend-opendal/src/lib.rs` — `ftp_operator`, and `classify`,
   which is string-matching an FTP status code and says so.
4. `tungstate-secret/src/lib.rs` — `EnvOverride`, thirty lines.
5. `tungstate-cli/tests/ftp.rs` — including the hand-rolled FTP client at the
   bottom, which exists so assertions do not go through the code under test.
