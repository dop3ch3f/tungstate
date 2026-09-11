# Tour: slice 4g

The brief is in [04g-stopping-and-sensing.md](04g-stopping-and-sensing.md).
Four Rust ideas here, and one of them — the condition variable — is the first
in this project.

---

## 1. An enum in a byte: `AtomicU8` as a state machine

A stop request has three states, and it is read on every chunk of every file
in flight. A `Mutex<Halt>` would be correct and would also put a lock on the
hottest path in the program, so `Stop` stores the enum as a number:

```rust
pub enum Halt { No, AfterThisFile, Now }

pub struct Stop(AtomicU8);
```

Rust will not let you atomically load a `Halt` — atomics exist only for
integers, pointers and `bool` — so the conversion is manual in both
directions. Out is `as u8`, which works because `Halt` is a *field-less* enum
and those have integer discriminants (`No` is 0, `AfterThisFile` is 1, `Now`
is 2, assigned in order). Back in is a `match`, and the catch-all matters:

```rust
match self.0.load(Ordering::Relaxed) {
    0 => Halt::No,
    1 => Halt::AfterThisFile,
    _ => Halt::Now,
}
```

There is no `_ => unreachable!()` because nothing needs to be unreachable. The
only writers are this module's own methods, so a 7 cannot appear — and if the
code were ever wrong, resolving an unknown value to "stop" is the failure you
want.

The interesting method is `raise`:

```rust
fn raise(&self, level: Halt) {
    self.0.fetch_max(level as u8, Ordering::Relaxed);
}
```

`fetch_max` stores the larger of the current and new value, atomically. That
is what makes "Stop now" survive a "Stop after these files" arriving a
millisecond later: the requests can race, and the race can only ever resolve
to the stronger one. Writing this as `if level > current { store(level) }`
would have a window between the read and the write where the other thread's
write is lost. The `#[derive(PartialOrd, Ord)]` on `Halt` is what lets the
test say `stop.level() > Halt::No` and mean it.

## 2. `Condvar`: the difference between leaving and waiting

Before this slice, a worker above the limit did this:

```rust
if index >= governor.limit() { return; }
```

That is a one-way valve. `std::thread::scope` spawned four threads; a returned
thread is gone, and if the limit went back up there would be nobody to take
the work. Fine when the limit only ever ramped *up* before the run. Not fine
once a measurement can be wrong and want its worker back.

So a worker parks instead:

```rust
let mut guard = self.parked.lock()...;
while index >= self.limit() {
    if done() { return false; }
    let (next, _) = self.room.wait_timeout(guard, Duration::from_millis(200))...;
    guard = next;
}
```

Three things are worth slowing down for.

**`wait` consumes the guard and hands it back.** In most languages a condition
variable is "unlock, sleep, relock" and you hope the caller remembers. Rust
encodes it in the types: `wait_timeout` takes the `MutexGuard` by value and
returns a new one. You *cannot* hold the lock while waiting, and you cannot
carry on without re-acquiring it, because the only guard you had was moved
into the call. The bug class disappears.

**It is a `while`, not an `if`.** A waiting thread can wake without anyone
having signalled — a spurious wakeup, permitted by every threading API that
has one. The condition must be re-checked on every wake. This is universal
across languages and is the single most common condvar mistake.

**The mutex guards nothing.** `parked: Mutex<()>` — the unit type. The state
being waited on is `limit`, which lives in an atomic because it is read
constantly and written rarely. That is unusual enough to deserve the comment
it has in the source: the mutex exists only because `Condvar::wait` requires
one, as the thing that makes "check the condition" and "go to sleep"
indivisible.

The 200 ms timeout is not a safety net for a missed signal. It is there
because the run can also end by the *queue emptying*, and nothing notifies
about that — so a parked worker has to wake occasionally and look.

## 3. A closure as a progress reporter, and `impl FnMut`

`digest` needed to report progress and to be interruptible, and it has two
callers that want different things — a content check spans two files and wants
one bar across both; a readback verification is its own thing. Rather than
give it a `&dyn Progress` and a pile of parameters describing the context:

```rust
fn digest_watching(
    backend: &dyn Backend,
    path: &Path,
    mut watching: impl FnMut(u64) -> bool,
) -> Result<String>
```

`FnMut` rather than `Fn` because the caller's closure mutates captured state:

```rust
let mut done = 0_u64;
let mut watching = |read: u64| {
    done += read;
    ...
    !self.abandoning()
};
```

The three closure traits are a hierarchy of what the closure does to what it
captured. `Fn` only reads. `FnMut` mutates — this one owns a running total.
`FnOnce` consumes, so it can be called once. Asking for the loosest one that
works is the habit: a `Fn` bound here would reject this closure outright.

Then it is passed as `&mut watching` to *both* digest calls. A closure is a
value, and `&mut F` implements `FnMut` when `F` does — so the same accumulator
survives across two calls and the bar crosses the whole check once rather than
filling twice. Passing `watching` by value to the first call would move it, and
the second call would not compile.

Returning `bool` for "keep going" keeps the abandon check in the caller, which
is the only place that knows what `abandoning()` means.

## 4. A variant that is not a failure

`TransferError::Stopped` sits in the same enum as `Io` and `Verification`, and
is deliberately handled nowhere near them:

```rust
Err(TransferError::Stopped { .. }) => {
    lock(shared).cancelled = true;
    return;
}
Err(error) => { /* count it, report it, carry on */ }
```

It travels as an error because it needs to unwind the same stack an error
unwinds — out of the chunk loop, out of `stream`, out of `copy_to` — without
every intermediate function growing a second return channel. But it is not a
fault, so it never reaches `summary.failures`, and in `copy_to` it is the one
error that does *not* mark the journal row `Failed`:

```rust
Err(TransferError::Stopped { .. }) => {}
```

Leaving the row `Intended` is the whole trick of the kill switch. `Intended` is
exactly what a killed process leaves behind, and slice 4d already built the
machinery that finds those rows, names them, and offers Resume and Clean up. So
the hard stop needed no recovery of its own — it only had to **fail in a shape
that recovery already recognises**. Worth noticing as a design habit: the
cheapest new feature is one that produces a state the system can already
handle.

## 5. What the measurement can and cannot promise

The ramp-down compares throughput at one width against throughput at the
previous one and keeps the narrower arrangement when it is within 5%. Two
honest limits:

**It cannot act mid-file.** A worker three gigabytes into a copy cannot stand
down without throwing that work away, so width changes land at file
boundaries. Over sixty files it converges in seconds; over four enormous files
it does nothing at all. That is a real gap and not a fixable one.

**Real throughput is noisier than 5%.** A window that happens to catch a slow
patch can read as "narrower was worse" and bounce the width back up. The
design absorbs this by *settling* after one bounce rather than continuing to
hunt: the worst case is that the run stays at the width the handshake chose,
which is where it would have been anyway. An oscillating governor would be
worse than no governor.

Both are why the window now shows the live number. A measurement you cannot
see is a measurement you cannot disagree with.
