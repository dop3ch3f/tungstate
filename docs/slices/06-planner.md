# Slice 6: the planner

**Goal:** the policy model can decide where one file belongs. Teach it to look
at a whole folder and say, in order, everything that would have to happen to
make the folder match its policy — while touching nothing.

**Runnable outcome:** `tungstate plan` on a messy folder. Every file accounted
for, either as an operation or as a skip with a reason; the operations in an
order that can actually be carried out; a blast-radius line saying how much of
the folder this would move; and the tree provably unchanged afterwards.

**This brief is the spec.** The tour will be in `06-tour.md`.

---

## What this slice is not

Slice 6 plans. **Slice 7 executes.** Nothing here writes to a governed folder,
and one consequence is worth stating up front because it shapes the op set:

> **No operation in a slice-6 plan removes content.** Not one. See "`Trash` is
> not needed" below.

---

## Decisions

### The index is deferred

**Chosen.** DESIGN §3 specifies a per-folder SQLite index holding path, size,
mtime, file id, hashes with the (size, mtime) they were computed at,
classification, last action and quarantine reason. Slice 6 does not build it.
It walks and classifies in one pass into an in-memory `Snapshot`.

**Why.** An index is a cache, and a cache earns its keep from *hits*. Nothing
reads one incrementally until dedup (slice 8) wants "find all files with hash X"
and the watcher (slice 9) wants "what changed since". Building it now ships the
whole of cache invalidation — the (size, mtime) staleness check, the
write-through on every plan, the "the file changed under us" race — in exchange
for nothing, three slices before the first reader exists.

It is also cheaper to skip than it looks. `Policy::required_tier()` means a
policy that never mentions `hash` never pays for one, so a plan over a video
library is `stat`-tier: a directory walk and no reading at all.

**Rejected: build it now, as DESIGN has it.** The cost is real and the benefit
is three slices away. **Rejected: a stub index that stores but is never read.**
That is the same code with the same invalidation bugs and a false claim in the
crate list.

The index arrives as `tungstate-index` (DESIGN §7 already reserves the name)
when slice 8 or 9 gives it a reader.

### A folder is found by walking up, not by a registry

**Chosen.** `tungstate plan [PATH]` finds the folder it governs exactly as
`explain` does: walk up from `PATH` to the nearest `.tungstate/policy.toml`, and
its directory is the root. Same function (`cli::folder::locate`), same rules,
same error message. No journal migration. `folder add` stays a stub.

**Why.** A registry is a list, and a list needs an iterator to be worth having.
The thing that iterates folders is the daemon's reconciliation loop, slice 10.
Adding a `folders` table now means a schema migration whose only reader is a
command that already has a better way to find its answer — and the policy file
*is* the registration, in the sense that matters: its presence is what makes a
directory a governed folder.

**Rejected: un-stub `folder add` here.** The 5b brief's wording ("stays a stub
until the planner gives a governed folder something to do") reads as a promise
to do it in this slice. Taken literally it would add a table with one reader.
The spirit of it — *a governed folder now does something* — is satisfied by
`plan` itself.

### Directory strictness is prune-only

**Chosen.** No new policy syntax. `strict` and `ensure` (DESIGN §2, deferred
past slice 5 pending an index) stay deferred. What the planner *does* do is
emit a remove-directory op for a directory **its own moves empty**.

**Why.** DESIGN §2 says empty directories are "pruned if tungstate created them,
left alone if the user did (the journal knows which)". Without a journal of
directory creation we cannot know who *made* an empty directory — but we know
with total certainty who *emptied* one, because it is written in the plan we
just computed. That is the knowable half, it needs no new state, and without it
the very first `plan` leaves a folder full of husks.

**Rejected: full `strict`/`ensure` now.** Policy syntax whose only consumer is
a dry run. It wants an executor to act on the drift it reports.
**Rejected: prune nothing.** A tidy-up that leaves the old shape behind in
outline is not a tidy-up.

### `Trash` is not needed, and that is a finding

**Chosen.** `on_conflict = "replace"` **quarantines** the existing file rather
than trashing it. Consequently slice 6 has no `Trash` op and no `Delete` op, and
nothing a plan can express removes content.

**Why.** DESIGN §5 says `replace` sends the existing file to the trash. The
shipped drain already decided otherwise, in
`crates/tungstate-transfer/src/lib.rs:952`:

> The existing file is quarantined rather than deleted. Replace is the user's
> decision about which copy they want, not permission to destroy the other one.

`--on-conflict replace` on a link and `on_conflict = "replace"` in a policy are
the same word offered to the same person about the same situation. If they meant
different things, the product would be lying with one of them. Two candidates
for which to change; the drain's version is already shipped, already argued in a
comment, and is the safer of the two.

The dividend is large for a plan-only slice: no reasoning about the OS trash,
`.tungstate-trash/` on remotes, retention, or DESIGN rail 2, and the "no content
is ever lost" invariant becomes trivially checkable rather than carefully
checkable. `Trash` arrives in slice 8 with `on_duplicate`, by which time an
executor exists to honour it.

**DESIGN §5 is amended in this slice**, with this reasoning.

### `plan` plans in every mode, including `observe`

**Chosen.** `tungstate plan` computes and prints whatever the folder's `mode`
says. The mode is carried on the `Plan` so slice 7's `apply` can refuse without
re-reading the policy, and the human output ends with a note when the folder is
in `observe`.

**Why.** DESIGN §4 says `observe`: "scan and report drift; never plan". That is
describing the daemon's loop, and it is loose wording, because **reporting drift
*is* planning** — the only way to know a file is misplaced is to compute where
it belongs and compare. What `observe` never does is *act*.

Put the other way: a mode is a statement about what tungstate may do to your
files, and `plan` does nothing to your files. Honouring it here would make the
safest command in the product the most restricted one, and would leave anyone
in `observe` mode — the default — with no way to see their drift at all.

**DESIGN §4 is amended** to read "never applies".

### Local folders only

**Chosen.** A `connection:path` target is refused, in the shape `explain` already
uses for the same refusal.

**Why.** `explain` requires `--policy` for a remote target because walking a
remote tree looking for `.tungstate/policy.toml` is a round trip per level, and
DESIGN §2 puts a remote folder's policy in the central config directory anyway.
Governing a remote folder in place is slice 13, with its own cost-aware
planning. No design debt is taken on: `survey` takes `&dyn Backend`, so the day
a remote folder's policy can be located, the planner works unchanged.

---

## What ships

1. **`tungstate-backend` gains `walk`** — a pruning, depth-first `Iterator` over
   a backend, one directory's listing at a time. `Backend::read_dir` is
   deliberately one level deep and its docstring already points at this: *"a
   single directory is bounded, so collecting it is honest; recursive walks need
   streaming."* It needs nothing but the trait, so it adds no dependency to a
   crate whose only one is `thiserror`.

   The prune predicate is a **closure, not a `Patterns`** — the backend crate
   must not learn the policy language. `tungstate-transfer::walk` keeps `sort`
   (it needs `journal::Order`) and `prune_empty` (it mutates, so it is executor
   work), and its `files`/`files_under` become adapters, leaving all 21 call
   sites untouched.

2. **`tungstate-core` gains `Snapshot`** — everything one pass over a folder
   saw, as of one moment: entries, the set of directories, and whether the
   backend is case-sensitive. Still zero I/O.

   Entries are full `Attributes`, not a slimmed verdict. That is what makes
   "apply the plan, then plan again" a pure one-line expression with no
   test-only back door, which is what the first invariant needs.

3. **`tungstate-attrs` gains `survey`** — the I/O half: walk, prune what the
   policy ignores, `gather` at the policy's tier, one `Timestamp::now()` for the
   whole pass so every file is classified as of one moment.

4. **`tungstate-core` gains `Policy::route`** — `explain` without the trace.
   `explain` evaluates every rule even after one matches and allocates a
   `RuleTrace` per rule, deliberately, because `AlsoMatches` is what makes the
   trace teachable. For one file that is the feature; for 200,000 files it is
   millions of discarded allocations. A property test asserts
   `route(a) == explain(a).outcome`, so the two cannot drift.

5. **`tungstate-core` gains the planner** — `Policy::plan(&Snapshot) -> Plan`,
   with four op kinds (`MkDir`, `Move`, `Quarantine`, `RmDir`), a reason for
   every untouched file, and a blast radius.

6. **`tungstate-core` gains the graph** — the dependency edges, a hand-rolled
   Kahn topological sort with a deterministic ready set, and cycle breaking by
   temporary name.

7. **`Plan::apply_to`** — the plan's meaning as a pure function, checking each
   op's precondition as it goes, so it proves the emitted order is *executable*
   rather than merely acyclic.

8. **`tungstate plan`** — `[PATH] [--policy FILE] [--json]`, human output in the
   house style `link preview` established, and the closing line *"Nothing has
   been changed."*

9. **`cli::folder`** — `locate`, `load`, the miette diagnostic and
   `describe_tier` move out of `explain.rs` so both commands share one way of
   finding and reporting on a policy.

---

## The op set

Four kinds, against DESIGN §4's nine.

| DESIGN | Slice 6 | Why |
|---|---|---|
| `MkDir` | **yes** | A rendered `path` routinely names a directory that does not exist. |
| `Move` | **yes** | The point. |
| `Rename` | folded into `Move` | A rename is a move whose parent is unchanged. Two variants would be two identical match arms for ever; the *renderer* says "rename" when the parents match, which is a display decision. |
| `Copy` | no | Cross-device only. One root, one backend. |
| `Link` | no | Exists to collapse duplicates. Slice 8. |
| `Trash` | no | See the decision above. |
| `Delete` | no | Must never be expressible in a plan (DESIGN rail 2). |
| `Quarantine` | **yes** | `quarantine` is the *default* `on_conflict`, so it is the common path. |
| `Noop(reason)` | not an op | Below. |
| — | **`RmDir`** | New. Decision 3. |

**`Noop` is not an op.** An op is a node in a graph of state changes; a noop has
no edges, no effect, and never goes away — so a plan containing them is never
"empty", and the invariant *apply it and replan and get nothing* becomes
unstatable. Per-file reasons live in `Plan::untouched` instead, which maps
one-to-one off the four non-`Routed` `Outcome` variants plus the four the
planner decides for itself.

---

## How a plan is built

**Phase 1, desire.** Per entry, in path order. Cooldown first — a file written
more recently than `[defaults].cooldown` is left alone, the same rule
`tungstate_transfer::within` applies, so both "what would happen" commands hold
files back for the same reason. Then the `Outcome`: routed and not in place is a
move; routed and in place is `AlreadyThere`; unmatched with an inbox goes to the
inbox, **unless it is already under it** — otherwise a file in `_inbox` plans
into `_inbox/_inbox/` and the second plan is not empty.

**Phase 2, two movers wanting one name.** Grouped by destination, case-folded
when the backend is not case-sensitive. The winner is the entry already at the
name, else the lexicographically smallest current path; the losers are numbered
`-1`, `-2` before the last extension, each candidate re-checked against movers
and non-movers alike. The rule that makes this survive a replan: **a candidate
occupied by the entry under consideration counts as free.**

**Phase 3, the name is held by something not moving.** A file: `on_conflict`
decides — number the mover, skip it, quarantine the occupant and move in, or
quarantine the mover. A *directory*: the file is left alone as `Blocked`.
`on_conflict` is about two files disagreeing over a name; silently quarantining
a whole directory tree is not what any of those four words promised.

**Phase 4, the directories.** `MkDir` per missing ancestor. `RmDir` for a
directory the plan's own moves empty and nothing lands in — **never for one that
was already empty**, which is the line between "tidy up after myself" and
"delete things I did not touch".

**Phase 5, the order.** Below.

---

## The graph

`u → v` reads *u must run before v*:

1. `MkDir(a/b) → MkDir(a/b/c)` — parent before child.
2. `MkDir(parent(to)) → Move|Quarantine(to)`.
3. `Op{from == p} → Move|Quarantine{to == p}` — **vacate before occupy.** The
   only class that can cycle.
4. `Op{from == d} → MkDir(d)` — a file moves out of the way before a directory
   takes its name.
5. `Op{from under d} → RmDir(d)` and `RmDir(d/child) → RmDir(d)`.

**Kahn's algorithm, hand-rolled.** Two reasons:

- **The residual is exactly the cycles.** Whatever still has a non-zero
  in-degree when the ready set empties is precisely the knot that needs breaking.
  A depth-first sort finds *one* back edge and leaves you to reconstruct the
  rest; `petgraph::algo::toposort` returns one node for the same reason.
- **Determinism is one line.** A `BTreeSet` of ready nodes rather than a queue
  means ties break on op content instead of insertion order, so the same
  snapshot always produces byte-identical output.

No `petgraph`: it would hide the one thing this slice exists to teach, and it
answers the cycle question in the wrong shape. **No `HashMap` anywhere in the
planner** — iteration order is the easiest way to make output irreproducible.

**Breaking a cycle.** Run Kahn; take the residual's lexicographically smallest
`Move` by source; rewrite it as a move to a temporary name plus a move from that
name to the real destination; re-run. The temporary is a **sibling of the
destination** — that directory provably exists, a rename within one directory is
the cheapest and most certainly-atomic thing any backend offers, and it needs no
scratch directory to create and clean up. A three-way rotation therefore costs
exactly **one** temporary name, not three. A residual with no `Move` in it
degrades to `Blocked` rather than looping: *never loop forever*.

---

## Acceptance criteria

- `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`
  and `cargo test --locked` clean.
- `tungstate plan` on a messy folder accounts for every file, as an op or as a
  skip with a reason.
- **It changes nothing**, proved by comparing the whole tree before and after.
- `tungstate plan --json` carries the same decisions as the human output.
- Two files that want to trade places produce exactly one temporary name; a
  three-way rotation also produces exactly one.
- The same folder planned twice gives byte-identical output.
- A folder large enough to trip the blast radius says so, and names the limit.
- Nothing is ever planned into `.tungstate/`.
- CI green on Linux, macOS and Windows.
- **By hand:** build a messy folder and read the plan, per the tour's walkthrough.

## Tests

**Property tests** — the four DESIGN §10 asks for, over the generators slice 5
already has:

- **Apply, replan, get nothing.** `policy.plan(&p.apply_to(&s)?).ops.is_empty()`.
  Reconciliation is idempotent and convergent, the invariant DESIGN §1 says to
  write on the wall.
- **Nothing lost, nothing duplicated.** With no `Trash` and no `Delete`, the
  multiset of entries is identical before and after. When `Trash` lands in slice
  8 this becomes (folder ∪ trash) and `Snapshot` grows a `trashed` list.
- **No op targets a path outside the root** — the existing escape test lifted
  from one destination to every op, with the same adversarial generators.
- **Cycle breaking worked** — expressed as `apply_to` returning `Ok`. Asserting
  the graph is acyclic tests the sort against itself; replaying the emitted
  order against a simulated filesystem tests it against what it is *for*.

**Unit tests** — each conflict action; each skip reason; a swap; a three-way
rotation; a directory emptied and pruned; a directory emptied but reused as an
ancestor and therefore not pruned; a file blocked by a directory; an inbox file
that stays put; numbering stable across a replan; `route` agreeing with
`explain`; nothing planned inside `.tungstate`.

**Snapshots** (`insta`) — the human rendering and the JSON, in core and through
the real binary. `help_output_is_stable` is re-accepted, read rather than blindly.

## Out of scope

- The SQLite index — slice 8 or 9, as `tungstate-index`.
- `apply`, `undo`, and the circuit breaker's *refusal* — slice 7. Slice 6
  measures the blast radius and warns; it cannot refuse what it never does.
- `strict` / `ensure` policy syntax.
- Dedup, hashing, `on_duplicate`, `Trash` — slice 8.
- Moving an opaque bundle as a unit. `pre_rules` returns `Outcome::Opaque`
  before any rule runs, so a `.app` is one untouched entry; moving it as a unit
  needs rules that match directories.
- Unicode normalisation in collision keys (DESIGN §6). Case folding is handled;
  NFC is not.
- Governing a remote folder in place — slice 13.
- A folder registry and `folder add` — slice 10.
