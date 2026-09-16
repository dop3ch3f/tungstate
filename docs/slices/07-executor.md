# Slice 7: the executor

**Goal:** carry out a plan, and be able to take it back.

**Runnable outcome:** `tungstate apply` on a messy folder actually tidies it,
recording every operation as it goes; `tungstate undo --last 1` puts it back
exactly as it was. Kill the process mid-apply and the next run knows what was
in flight.

**This brief is the spec.** The tour will be in `07-tour.md`.

---

## What this slice is, in one sentence

Slice 6 said what would happen and changed nothing. This one changes things —
which means every safety rail in DESIGN §4 stops being a note and becomes code.

---

## Decisions

### The executor is its own crate

**Chosen.** `tungstate-execute`, depending on `tungstate-core` (for `Plan`),
`tungstate-backend` and `tungstate-journal`.

**Why not `tungstate-transfer`.** That crate is about *links*: two backends, a
verify level, a resume, a conflict policy per link. This is about one folder,
one backend, and a plan somebody already approved. They share a discipline
(journal the intent, act, journal the outcome) and almost no code. Putting both
in one crate would make it the home of two engines that happen to rhyme.

**Why not the CLI.** Slice 10's daemon runs reconciliation loops and needs this
without `clap`, and slice 5c's window will want it too.

### Journal migration v7: a plan is a thing that happened

**Chosen.** A `plans` table, and a `plan_id` on `ops`:

```sql
CREATE TABLE plans (
    id         INTEGER PRIMARY KEY,
    folder     TEXT    NOT NULL,   -- the root, as applied
    snapshot   TEXT    NOT NULL,   -- the fingerprint it was built from
    applied_at INTEGER NOT NULL,
    undone_at  INTEGER             -- NULL until it is taken back
);
ALTER TABLE ops ADD COLUMN plan_id INTEGER REFERENCES plans (id);
```

Additive, so every existing row is correct with no backfill — `plan_id IS NULL`
means "a drain did this, not a reorganisation", which is exactly true.

**A table rather than a bare `plan_id` string**, because three questions need it
and none is answerable from a column alone: *what were the last three
reorganisations* (`undo --last N`), *which folder was this* (the ops only know
paths), and *has this already been taken back* (`undone_at`). DESIGN §4c already
puts `plan_id` in the op record; this is that, plus the row it points at.

`OpKind` gains **`RmDir`**. `MkDir` was in the journal from slice 2 and has
never been written by anything; both are now.

### Journal the intent, act, journal the outcome — per operation

**Chosen.** The same ordering the drain has used since slice 3, for the same
reason. `begin()` writes `intended` *before* the filesystem is touched;
`finish()` records what happened. Anything still `intended` at startup was in
flight when the process died.

**The recovery rule is different from the drain's, and simpler.** A drain's
interrupted op may have left a half-copied temp file. A reorganisation's ops are
single renames, which either happened or did not — so recovery is a question,
not a repair: *is the source still there, or is the destination?* Whichever it
is, the row is closed honestly and the next plan sees the truth. **No plan is
ever resumed.** Replanning is cheap and convergent, so continuing a half-applied
plan buys nothing and risks acting on a stale view.

### Stale plans are refused, at two scales

**Chosen, both.** DESIGN §4: *stale plan → replan, not stale action.*

1. **Before the first op**, the folder is surveyed again and the fingerprint
   compared with `plan.snapshot`. Different means the folder moved since the
   plan was made — refuse, and say to replan. This is what makes a saved
   `plan.json` safe to apply later.
2. **Before each op**, the source is re-stat'd. If its size or modification time
   has changed since the snapshot, that one op is skipped and recorded as
   skipped. Someone is editing the file right now; moving it out from under them
   is the rudest thing this program could do.

### A failure stops the run only when the failure means the plan is wrong

**Chosen.** Two classes, because DESIGN gives two different instructions and
both are right.

- **The world is not as the plan believed** — the source has gone, the
  destination is occupied, the root has changed identity. The plan is stale, so
  the rest of it is guesswork. **Stop**, report, and say to replan.
- **This particular operation could not be done** — permission denied, a file
  locked by another process. **Record it as failed and carry on.** DESIGN §9:
  *never abort the whole cycle for one bad directory.*

The half-applied result is not a problem, and that is worth saying out loud:
reconciliation is convergent, so a run that stops halfway leaves a folder that
the next `plan` describes correctly. This is the property that pays for itself
in every failure path in the slice.

### The circuit breaker refuses two things

**Chosen.** `apply` refuses without `--yes` when either holds:

- `blast.over_limit` — more than 500 files, or a fifth of the folder (DESIGN
  rail 3). The limit already lives in `core::plan::Blast` so `plan` and `apply`
  cannot disagree.
- `!plan.settles` — the policy does not converge. Slice 6 detects it; this is
  the slice where ignoring it starts to cost something.

Both are `--yes`-able, because both are judgements about *your* files and the
program's job is to be sure you meant it, not to know better.

### Undo reverses journalled operations, and refuses when it cannot

**Chosen.** `tungstate undo [--last N | --plan <ID>]`.

The inverse of each op is obvious and total: `Move(a → b)` reverses to
`Move(b → a)`, `MkDir(d)` to `RmDir(d)`, `RmDir(d)` to `MkDir(d)`, and a
`Quarantine` is a `Move`. Applied in **reverse order**, which is what makes the
directory ops line up — the last thing created is the first thing removed.

**Undo is checked, not forced.** Before reversing `Move(a → b)`, `b` must still
be where the journal left it and `a` must still be free. If either has changed,
the undo stops and says which file and why. Forcing would mean overwriting
whatever the user has since put at `a`, and an undo that destroys work is worse
than no undo at all.

The inverse operations are journalled in their own right, under a new plan, and
the original is marked `undone_at`. So undo is visible in `tungstate log` like
everything else, and undoing the same plan twice is refused rather than
silently redoing it.

**Rejected: `undo --since <time>`.** DESIGN §4 mentions it and §8 does not.
`--last N` and `--plan <ID>` cover what anyone actually reaches for, and a time
window over a table of plans invites "undo everything since Tuesday", which is
the request most likely to be regretted.

### The root's identity is pinned, as it is for a drain

**Chosen.** `Backend::root_token()` before the first op, re-checked before each
one, exactly as the drain does. This is the invariant that exists because a
vanished mount once put 572 MB on the wrong disk and deleted the originals. A
governed folder on an external drive is the same risk wearing different clothes.

---

## What ships

1. **Journal** — migration v7 (`plans`, `ops.plan_id`), `OpKind::RmDir`,
   `begin_plan`, `finish_plan`, `ops_for_plan`, `recent_plans`, `mark_undone`.
2. **`tungstate-execute`** — `apply(plan, backend, journal) -> Applied`, with
   per-op journalling, the two staleness checks, the two failure classes, and
   the root token.
3. **`invert`** — a plan's operations turned into the plan that undoes them,
   in `tungstate-execute`, pure enough to test without a filesystem.
4. **`tungstate apply [PATH] [--yes] [--plan FILE]`** — replans and applies, or
   applies a saved plan after checking its fingerprint.
5. **`tungstate undo [--last N | --plan ID]`** — with a dry-run summary before
   it acts, because an undo is exactly when someone wants to be sure.
6. **Crash recovery** — `intended` rows for plan operations are resolved by
   asking the filesystem which side of the rename won, on the next `apply` or
   `undo` of that folder.

---

## Acceptance criteria

- `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`
  and `cargo test --locked` clean; CI green on Linux, macOS and Windows.
- A messy folder, `tungstate plan`, then `tungstate apply --yes`: the folder
  matches what the plan said, and a second `plan` finds nothing to do.
- `tungstate undo --last 1` returns the folder to exactly its previous shape —
  compared by a census of every path, size and modification time.
- `apply` refuses a plan past the blast radius without `--yes`, and names the
  limit.
- `apply` refuses a plan whose policy does not settle without `--yes`.
- A `plan.json` saved, the folder then changed, and `apply --plan` refusing it
  as stale.
- A file edited between plan and apply is skipped, not moved, and says so.
- `kill -9` mid-apply, then `tungstate apply` again: the interrupted operation
  is resolved honestly and the folder ends up correct.
- **By hand:** reorganise a real folder of throwaway files and put it back.

## Tests

**Property tests** — applying a plan to a real temp directory and surveying
again gives the same snapshot `Plan::apply_to` predicted. That is slice 6's
paper model used as the executor's specification, and it is the highest-value
test in the slice: one assertion covering every op kind and every ordering.

Plus: undo after apply restores the original census, over generated folders.

**Unit tests** — each op kind applied and inverted; a stale fingerprint refused;
a changed source skipped; a permission failure recorded without stopping the
run; a vanished destination stopping it; double-undo refused; recovery resolving
an `intended` rename from either side.

**CLI tests** — the full round trip through the real binary, and the two
refusals with their messages.

## Out of scope

- `purge`, and real deletion of anything. Trash arrives with dedup in slice 8.
- Dedup, hashing, `on_duplicate`.
- The watcher and the daemon — slices 9 and 10. This applies a plan when told
  to, once.
- Remote folders — slice 13.
- `undo --since`, decided above.
- Parallel execution. DESIGN §4 allows four at a time; a reorganisation is
  renames, which are fast and ordered by a graph that assumes one at a time.
  Revisit when a real folder is slow, not before.
