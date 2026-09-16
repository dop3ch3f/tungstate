# Tour: slice 7

The brief is in [07-executor.md](07-executor.md).

One new crate, one migration, and two commands. The Rust here is mostly things
you have met; what is worth slowing down for is a set of decisions about
*failure*, because this is the first slice where being wrong costs somebody
their files.

---

## 1. Why this is not part of `tungstate-transfer`

The drain already does the hard version of this. It journals an intent, writes
to a temporary name, verifies, renames, journals the outcome, and only then
touches the source. A reorganisation journals an intent, renames, and journals
the outcome. The second looks like the first with pieces removed, and the
obvious move is to put it in the same crate.

It is a different crate, and the reason is what each one is *about*:

- `tungstate-transfer` is about a **link** — two backends, a verify level, a
  resume, a conflict policy, an order, a cooldown, all stored per link.
- `tungstate-execute` is about a **plan** — one folder, one backend, and a list
  of operations somebody already approved.

They share a *discipline* and almost no code. Put both in one crate and it
becomes the home of two engines that happen to rhyme, which is how a crate
turns into a module named `util`. The test is not "do these look similar" but
"would a change to one ever need to be a change to the other", and the answer
is no.

---

## 2. Journal before act, and the recovery that is a question rather than a repair

The ordering is the drain's, and it has not changed since slice 3:

```rust
let id = journal.begin(&NewOp { .. })?;   // written down first
match perform(op, backend) {              // then done
    Ok(()) => journal.finish(id, &Outcome::Committed { hash: None })?,
    Err(e) => journal.finish(id, &Outcome::Failed { error: describe(&e) })?,
}
```

Anything still `intended` when the process starts again was in flight when it
died. What is *different* here is what you do about it.

A drain's interrupted operation may have left a half-copied temporary file at
the destination, so recovering one is a **repair** — find the temp, decide
whether it is complete, delete or resume. A reorganisation's operations are
single renames. A rename either happened or it did not; POSIX gives no third
state. So recovery is a **question**, asked of the filesystem:

```rust
match (exists(op.source), exists(op.destination)) {
    (false, true) => Resolution::Committed,   // it went through
    (true, false) => Resolution::Abandoned,   // it never started
    _             => Resolution::Unclear,     // somebody else's doing
}
```

and the answer closes the row honestly. `Unclear` is recorded as failed rather
than guessed at, because a rename cannot leave both sides present, so if both
are there something outside tungstate did it and guessing means guessing about
someone's files.

**Nothing is retried, and no plan is ever resumed.** That is the interesting
part, and it is convergence paying for itself again: replanning is cheap, and
the next `plan` simply describes whatever shape the folder is actually in. A
resume would be code that exists to avoid work that costs milliseconds, and it
would have to reason about a half-applied ordering — which is exactly the state
the dependency graph was built to avoid being in. There is a test for the
principle rather than the mechanism:

```rust
#[test]
fn a_partly_applied_folder_is_described_correctly_by_the_next_plan()
```

---

## 3. Two staleness checks, at two scales

DESIGN §4 says *stale plan → replan, not stale action*. That turns out to be two
different checks.

**The whole plan.** Before the first operation, the folder is surveyed again and
its fingerprint compared with the one the plan was built from:

```rust
if tungstate_core::plan::fingerprint(fresh) != plan.snapshot {
    return Err(ExecuteError::Stale { .. });
}
```

This is what makes `tungstate plan --json > plan.json`, read it over coffee,
`tungstate apply --plan plan.json` a safe thing to offer. Add one file in
between and it is refused.

Note `fingerprint` had to become `pub`. It was private to slice 6, and the
executor needs to compute it over a *fresh* snapshot — two implementations of
"what counts as the same folder" is exactly the kind of near-agreement that ends
with a stale plan being applied.

**One operation.** Before each rename, the source is re-stat'd and compared with
what the snapshot recorded:

```rust
if now.len != expected.size { return Some(format!("it was {} bytes when planned and is {} now", ..)); }
```

Someone may be editing that file *right now*. Moving it out from under them is
the rudest thing this program could do, so that one operation is skipped and
reported and the rest of the run carries on.

The test for this does not sleep:

```rust
// Pretend `a.txt` was 99 bytes when planned, so the re-stat disagrees --
// the same thing an editor saving the file would produce, without a sleep.
```

A test that sleeps is a test that asserts on the machine it happens to run on —
a lesson this project learned in slice 4g and keeps.

---

## 4. Two failure classes, because DESIGN gives two instructions

Both of these are in DESIGN, and both are right:

> §4: stale plan → replan, not stale action.

> §9: Permission denied on a subtree: log, mark it, continue. **Never abort the
> whole cycle for one bad directory.**

They contradict each other until you notice they are about different failures.

```rust
fn fatal(error: &BackendError) -> bool {
    match error {
        BackendError::RootUnreachable(_) | BackendError::PathEscapesRoot(_)
        | BackendError::PathNotRelative(_) => true,
        BackendError::Io { source, .. } => matches!(
            source.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::AlreadyExists
        ),
        _ => false,
    }
}
```

`NotFound` means the source the plan expected has gone. `AlreadyExists` means
the destination the plan believed was free is not. Both are statements about the
**plan**, so everything after them is guesswork — stop. Permission denied is a
statement about **this file**, so record it and move on.

And the reason stopping is acceptable at all is the property from slice 6: a
half-applied folder is exactly what the next `plan` describes. There is nothing
to clean up, nothing to resume, and nothing lost. Convergence is not a nice
theoretical property here — it is what makes every failure path in this slice
short.

---

## 5. Undo, and the check that had to be someone else's

The inverse of each operation is total, which is the dividend of keeping the op
set to four:

```rust
(OpKind::Rename | OpKind::Move, Some(from), Some(to)) => Op::Move { from: to, to: from, .. },
(OpKind::MkDir, Some(path), _) => Op::RmDir { path },
(OpKind::RmDir, Some(path), _) => Op::MkDir { path },
```

applied over `.rev()`, and filtered to `status == Committed` — a failed or
skipped operation changed nothing, so undoing it would be inventing work.

**Reverse order is not a detail.** The last directory created has to be the
first removed, or a parent is not empty when its turn comes. There is a test
that asserts the *order*, not just the set.

### The check I got wrong

Undo has to be refused as a whole, before anything moves — finding out half way
leaves the folder in a third shape nobody asked for. My first version walked the
inverted operations and asked the filesystem about each:

```rust
Op::RmDir { path } => {
    if backend.read_dir(path).is_ok_and(|e| !e.is_empty()) { return Err(Stale { .. }); }
}
```

and it failed immediately: *`Video` is no longer empty, so undoing would not be
safe*. Of course it is not empty — it has the file in it. The undo's **own
earlier move** is what empties it. I was asking about the folder as it is now,
and the question is about the folder as it will be by the time that step is
reached.

That question already had an implementation. Slice 6's `Plan::apply_to` walks a
list of operations against a snapshot, checking each precondition as it is
*reached*. So `apply_to` grew a sibling:

```rust
pub fn replay(ops: &[Op], before: &Snapshot) -> Result<Snapshot, PlanError>
```

and `Plan::apply_to` became one line of it. Undo now asks the planner's own
question — *is this order executable against this folder?* — instead of a
weaker one it invented. One rule, two callers.

That is the third time in three slices that shape has been the answer:
`ConflictAction::wants_asking` in 5b, `Blast::LIMIT_FILES` in 6, `replay` here.
The recurring smell is **a second caller re-deriving a rule the first one
already owns**, and the fix is always to move the rule, never to copy it.

---

## 6. The bug that ran the program backwards

Everything passed. Then, by hand:

```
$ tungstate undo /tmp/tw7b
undid plan 1: 4 operation(s)
$ tungstate undo /tmp/tw7b
undid plan 2: 6 operation(s)     # <- redid the reorganisation
```

An undo journals its inverse operations **as a plan**. It has to — otherwise
its work never appears in `tungstate log`, and history quietly rewrites itself.
But that makes it indistinguishable from ordinary work, so `undo --last` picked
up the undo and reversed *that*.

The brief had named this exact hazard — *"undoing an undo is a redo wearing the
wrong name"* — and guarded it, at the wrong level. `mark_undone` refuses a
second undo **by plan id**, which is the half `--plan` uses. `--last` does not
name an id; it *chooses* one, and the choosing had no rule.

```sql
undoes INTEGER REFERENCES plans (id)
```

plus one predicate that both the selection and the tests read:

```rust
pub fn is_undoable(&self) -> bool {
    !self.is_undone() && !self.is_an_undo()
}
```

Two things worth taking from this.

**A guard on the identifier is not a guard on the choice.** Any command with
both a "by name" and a "pick one for me" form has two paths to the same action,
and the safety rule has to sit where they meet.

**The column went into migration v7 rather than a new v8.** The rule that
migrations are append-only protects *released* builds, and v7 had never been
released — it was two commits old and unpushed. A v8 that exists only because I
forgot a column in v7 is worse archaeology than a complete v7.

---

## 7. The circuit breaker is in the CLI, deliberately

`tungstate-execute::apply` does not check the blast radius. It does what it is
told.

```rust
/// The circuit breaker is **not** applied here. Whether a blast radius or a
/// policy that never settles should stop the run is a question for whoever is
/// talking to the user; this function does what it is told, and the CLI is
/// where `--yes` lives.
```

`--yes` is a *conversation*, and a library that cannot have one should not be
deciding how it goes. The daemon in slice 10 will make the same decision
differently — it has nobody to ask, so for it the breaker is a hard refusal and
a notification. Both are reading the same number, `Blast::LIMIT_FILES`, which
lives in `tungstate-core` so `plan` and `apply` cannot disagree about what "too
much" means.

The messages name the limit rather than implying it:

```
refusing: this would move 4 of 6 file(s), past the 500-file / 20% limit.
Run `tungstate plan` and read it, then pass --yes if that is what you meant.
```

DESIGN's argument for rail 3 is *"what saves you when you typo the policy and it
wants to reshuffle 80k photos"*, and being saved means being told the number.

---

## 8. One more honest message

A run refused before it started said:

> Nothing after that point was attempted.

True, and useless — there was no point. One `ExecuteError::Stale` was covering
two situations that need different sentences, so it became two:

```rust
Stale { what }                              // found up front; nothing changed
StoppedPartWay { plan, done, what }         // carries what was done
```

`StoppedPartWay` carries the plan id and the count, because the right next move
depends on them:

```
Those 3 operation(s) are recorded. Take them back with `tungstate undo --plan 4`,
or run `tungstate plan` to carry on from here.
```

An error type is a vocabulary. When one variant needs two different follow-up
sentences, it is two variants that have not been written yet.

---

## What this leaves

The reconciliation half now goes all the way round: declare a shape, see what
would change, change it, and take it back. What it does not do yet is notice on
its own — slice 8 is duplicate handling, and slice 9 is the watcher that makes
this continuous rather than something you type.

The `kill -9` path is covered by tests that construct the interrupted state
directly rather than by racing a real process. A rename completes faster than a
signal arrives, which is the same reason slice 4e could not kill a loopback FTP
drain mid-file. Worth saying plainly rather than claiming a test that is really
a coin flip.
