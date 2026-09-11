# Slice 4 tour: concurrency, sensed rather than configured

Read this next to the diff. Two ideas in it came from the user rather than
from me, and both were better than what I had.

Files: `tungstate-transfer/src/governor.rs` is new. `lib.rs` gained a worker
pool and lost its `&mut self` from everything per-file. `Progress` and
`ConflictResolver` gained `Send`.

---

## Part 0: Two corrections, recorded

**"Auto-sensing by connection, so someone cannot set 16 threads for an FTP
connection to a NAS."** My first design was a per-scheme default plus back-off
on error. Right instinct, wrong emphasis: the number should be a property of
the *connection*, not a global anyone can set.

**"Detect the transfer limits first… so some files do not have to be the
scape-goat and fail."** This is the one that changed the design. Back-off
learns by failure. Nothing is lost when it does — the source is untouched and
the journal records it — but the run reports "3 files failed" when nothing was
wrong, and a part-sent gigabyte is discarded to learn something a handshake
answers instantly.

The hazard is not hypothetical. `OpenDAL`'s FTP pool is sized **64**, and
its own source documents the reply:

```rust
// `{ status: NotAvailable, body: "421 There are too many connections from your internet address." }`
```

We build without the retry layer, so that lands as a straight failure.

## Part 1: Asking costs a handshake, not a file

```rust
fn accepts_another(destination: &dyn Backend) -> bool {
    let name = format!(".tungstate-slot-{}", std::process::id());
    match destination.stat(Path::new(&name)) {
        Err(BackendError::Io { source, .. }) if source.kind() == NotFound => true,
        Err(_) => false,
        Ok(_) => true,
    }
}
```

The inverted-looking arm is the interesting one. **`NotFound` is a success.**
The question is not "is there a file called that" — there certainly is not —
it is "did the far side let me open a connection to find out". A refusal is
anything that is not a clean answer.

Nothing is written, so the worst case is a line in somebody's server log. That
is the difference between this and the alternative considered: transferring
sample files at increasing concurrency measures the data path more directly,
but it writes into a destination to calibrate, leaves litter if it dies
mid-probe, and spends real seconds before any of the user's files move.

The climb happens **before the first file**, not during the run:

```rust
loop {
    let before = governor.limit();
    if governor.try_promote(self.destination) == before { break; }
}
let workers = governor.limit().min(files.len().max(1));
```

Ceilings are low, so this is at most two or three handshakes. Doing it up
front also means the number is known before anything moves, which is what
makes it reportable.

## Part 2: Two things deliberately not built

**Throughput-based stopping.** Climbing while throughput improves and halting
at the plateau is what the adaptive-concurrency literature does, and I wrote it
into the brief before taking it out. Throughput across files of wildly
differing sizes over a NAS is noisy, and a bad measurement makes a run slower
while looking principled. The ceilings here are 4–8; overshooting is not the
failure worth engineering against. Revisit with numbers from a real drain.

**Remembering the limit between runs.** Decided against separately: it needs a
schema column, and a wrong guess then persists until someone works out how to
clear it. A probe is two handshakes. Re-learning is cheaper than explaining.

## Part 3: `&mut self` was the thing in the way

Every per-file method was `&mut self`, because two fields needed mutation:

```rust
resolver: &'a mut dyn ConflictResolver,
progress: &'a mut dyn Progress,
```

Putting both behind a `Mutex` turned the entire per-file path from `&mut self`
into `&self`, which is what let workers share one `Transfer`:

```rust
resolver: Mutex<&'a mut dyn ConflictResolver>,
progress: Mutex<&'a mut dyn Progress>,
```

`Mutex<T>: Sync` when `T: Send`, so adding `Send` to the two traits made
`Transfer` shareable. Nothing else needed touching: `Backend` was already
`Send + Sync`, and `Journal` is `Sync` because its connection lives behind a
mutex — a decision from slice 2 that quietly paid for itself here.

At the call site the reborrow is explicit and worth a comment:

```rust
// A shared reborrow: every worker needs `&Transfer`, and `carry` holds `&mut`.
let me: &Self = self;
std::thread::scope(|scope| {
    for index in 0..workers {
        scope.spawn(move || me.work(index, queue, shared, governor, anchor));
    }
});
```

`std::thread::scope` rather than `Arc` everywhere: scoped threads are
guaranteed to finish before the scope ends, so they may borrow from the stack.
That is what makes `&Link`, `&dyn Backend` and `&Journal` usable directly
instead of being wrapped.

**Shrinking a running pool.** Threads cannot be un-spawned, so each worker
carries an index and checks it:

```rust
if index >= governor.limit() { return; }
```

A worker whose index falls outside a reduced limit finishes what it holds and
leaves. No signalling, no condition variable.

## Part 4: The race I did not predict

I guarded `disambiguate` against two workers choosing the same free name, then
wrote a test for it — and the test was unwritable. Destinations mirror the
source tree, so `src0/clash.mp4` and `src1/clash.mp4` land in different
directories. Two files can never contend for one name by accident.

The reachable race is different and I only found it by trying to build the
unreachable one:

> `holiday.mp4` clashes, so it is renamed to `holiday-2.mp4` — while another
> worker is transferring a source file genuinely called `holiday-2.mp4` to
> exactly that name.

Sequentially, whichever goes first is seen by the second. In parallel both
probe an empty slot and one silently overwrites the other. **Concurrency
created this one**, and the cost is a lost file, which is the one thing this
project exists to prevent. The test failed before the fix:

```
`a real file of that name` was overwritten; the destination holds
{"already here, and different", "the renamed one"}
```

The fix is possible only because of slice 4f. The engine now knows the whole
plan before it starts, so it can claim every destination up front:

```rust
let mut claimed = lock(&self.claimed);
for file in &files {
    claimed.insert(file.path.clone());
}
```

A rename then cannot choose a name that a file still waiting is going to need.
Two slices apart, and the second one paid for the first.

## Part 5: Cancelling means something slightly different now

The old test asserted `transferred == 1` after cancelling. With workers that is
no longer meaningful, and re-baselining the number would have missed the point.
What cancellation promises is not a count:

```rust
assert_eq!(
    usize::try_from(summary.transferred).unwrap_or(usize::MAX) + left,
    24,
    "{} moved and {left} left does not account for 24",
);
```

Every file is either moved or exactly where it was, and no partials survive.
The button's label changed too — "Stop after this file" is now "Stop after
these files", because that is what it does.

## Part 6: Measuring, and being honest about the measurement

The first benchmark was 24 files of 4 MB over FTP to a container on loopback:

```
sequential     0.7s   129.5 MB/s
auto           0.5s   178.0 MB/s
--parallel 4   0.5s   197.0 MB/s
```

Those numbers are close to worthless. Half a second is noise, and loopback has
no latency for concurrency to hide — the bottleneck is memory bandwidth.
Quoting "1.5× faster" from that would have been dishonest.

The shape a NAS actually presents is many files and real round trips:

```
300 files x 30 KB over FTP
sequential     5.0s      60 files/s
auto (2)       2.1s     144 files/s
--parallel 4   1.9s     154 files/s
```

**2.4× at the auto-sensed limit**, and the jump from 2 to 4 is small — which is
evidence for the low ceilings rather than against them.

## What to look at in the diff

1. `governor.rs` — the whole file, and `accepts_another` in particular.
2. `lib.rs`, the `Transfer` struct — two `Mutex` fields, and the comment on
   `claimed` explaining a race that has already bitten.
3. `lib.rs`, `work` — one worker, and the index check that is how a pool
   shrinks.
4. `tests.rs`, `a_renamed_file_never_lands_on_a_name_another_file_is_about_to_use`
   — the bug, written down.
