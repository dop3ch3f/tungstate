# Slice 9b: folder sync

**Goal:** a set of folders on any number of machines can be kept in step, in
the direction the user chooses, with nothing lost and everything the sync did
reversible. The drain moves a file once; this remembers what it moved and
keeps the set true afterwards.

**Runnable outcome:** four, one per sub-slice, because this brief covers 9b
to 9e.

- **9b.** `tungstate sync add capcut ~/CapCut nas:capcut --all`, then
  `tungstate sync run capcut`: new files on the laptop go to the NAS and new
  files on the NAS come to the laptop. Edit one file, run again, and only that
  file moves. Run a third time and nothing moves. Add `ftp:capcut` as a third
  member and the next run fills it from whichever member is cheapest to read.
- **9c.** The same sync with `--exact`: delete a project on the laptop and the
  next run sets the NAS copy aside; edit the same file on both and the run
  parks the other member's version beside it and says so. `tungstate undo
  --last 1` puts every member back. `tungstate sync forget capcut old.mp4`
  removes a file from every member in one go, and it does not come back.
- **9d.** All of it in the window: a Sync tab, the members in a row, what
  would move drawn per member before the button, a conflict asked about with
  apply-to-all, and a sync marked to run when the app opens.
- **9e.** With the window open, save a file into `~/CapCut` and watch it arrive
  on the NAS after the cooldown, with no button pressed.

**This brief is the spec.** The tours will be in `09b-tour.md` to
`09e-tour.md`, one per sub-slice.

---

## Why this exists

The case that prompted it: a video editor writes finished exports into one
folder on the laptop. The NAS holds the archive of every export ever made,
including ones the laptop no longer has. The user wants, every time the app
opens, the laptop's new exports on the NAS and the NAS's older exports on the
laptop, so the two are the same folder in two places. Sometimes the NAS is
reachable as a mount, sometimes only over FTP, and a second machine may join
later. Nothing in the product does this today.

`--copy` is the nearest thing and it is not this. A copy link walks its source
and asks the destination one question per file: *is this already there?* It
never notices a file that exists only at the far end, never notices that the
far end edited a file it once delivered, and never learns that a file it
delivered has since been deleted at the source. It cannot tell *new here* from
*deleted there* because it has no memory of what it saw last time. A sync is a
link with memory, and once it has memory it can run in any direction.

The user's four behaviours, in their words, are: *mirror both* (every member
ends up with everything), *right sync* (everything on the laptop must be on
every destination, not the reverse), *left sync* (everything on any destination
must be on the laptop), and *1-1 match* (adds, edits and deletes propagate
everywhere, like a cloud sync client but between machines you own). They
asked for it across *any number* of destinations configured together: two
folders on one laptop, a laptop and a NAS, a laptop and a NAS and an FTP
server, later SMB and cloud services.

**On "sync, not structure".** DESIGN's introduction sets tungstate apart from
rclone and Syncthing with that phrase. It still holds. Those tools synchronise;
they have no notion of a desired shape, so nothing to converge to and nothing
to undo. What this slice adds is a *declared* desired state of a different
kind: not the shape of one tree but the equality of several. "These folders
hold the same things" is a statement about desired state exactly as "photos go
under their year" is, the diff is computed the same way, the plan is previewed
and journalled the same way, and it is reversible for the same reason. DESIGN
§4d records the qualification.

## Why it is 9b, and what 8 means

It needs two things that exist by then. The dedup slice brings the set-aside
area and the `Trash` op, which is where a superseded or deleted copy goes when
the sync is told to remove it; without that, exact sync would have to invent
removal on its own, and DESIGN rail 2 says removal is one mechanism, not
several. The watcher (slice 9) brings change notification for local members,
which 9e turns into continuous sync while the window is open. It does not need
the daemon (10): an on-demand, on-launch and window-open sync is useful on its
own, and the daemon later triggers the same function unattended.

A note on numbering. The syllabus table still calls dedup "slice 8", and
`docs/slices/08-the-redesign.md` has since taken that file name. This brief
refers to *the dedup slice* by name where it matters and assumes it has a
number of its own before it starts. That is a table to fix, not a dependency
to argue about.

---

## Decisions

### A sync is a set of members, not a pair

**Chosen.** `tungstate sync add <name> <end> <end> [<end>...]`, two or more
members. A member is an end in the grammar the whole product already uses,
`~/CapCut` or `nas:capcut`, plus a short name: the connection's name by
default, `here` for a local path, `--as <name>` to choose. Member names appear
in the preview, in the names of parked conflict copies and in every "left
alone" reason, so they have to be words a person picked rather than paths.

**Why not pairs.** Every pair of members would need its own baseline and its
own conflict history, and three members would be three syncs that can
disagree about one file. A set has one desired content per path and every
member is measured against it. The user also asked for exactly this.

**Rejected: a hub-and-spoke of pairwise syncs.** It reaches the same members
with more objects, and it makes *the laptop* the centre by construction, which
is wrong for `--all`, where no member is special.

### Two knobs, not four named modes; push and pull have an anchor

**Chosen.** A direction, `--push | --pull | --all`, and a switch, `--exact`.
The four behaviours the user named are `--push`, `--pull`, `--all` and
`--all --exact`. The two exact one-way mirrors come free.

| direction | without `--exact` | with `--exact` |
|---|---|---|
| `--push` (the anchor sends) | every member ⊇ anchor: *right sync* | every member == anchor; a member's extras and edits are removed |
| `--pull` (the anchor receives) | anchor ⊇ every member: *left sync* | anchor == union of members; anchor-only files and anchor edits are removed |
| `--all` | every member == union of all: *mirror both* | *1-1 match*: adds, edits and deletes propagate to every member |

`--push` and `--pull` need to know which member is *the laptop*: the
**anchor**, the first member listed unless `--anchor <name>` says otherwise.
`--all` has no anchor and refuses one.

**Why two knobs.** Four names hide two independent choices, and a reader who
has learned `push` and `exact` can predict `push --exact` without a table.
Stored as two columns, `direction` and `exact`, and rendered by the window in
plain words (9d decides which).

### A leg is a link

**Chosen.** Bytes move between exactly two members at a time, through
`tungstate-transfer` with `SourcePolicy::Keep`, as a link that the sync owns:
`links.sync_id` is set, `saved` is false, and the Links tab hides it the way it
already hides one-off browser transfers. A leg is created the first time an
ordered pair of members needs to move a file and reused after that. Everything
the drain has earned is inherited without a line of engine change: the
temp-name-verify-commit order, per-file resume after a crash, the no-rename
commit path for FTP, quarantine, progress events, **Stop now**, the governor,
and interrupted-work recovery keyed by link.

Between two remote members there is no server-side copy across services, so a
leg from `nas:` to `ftp:` reads through this machine and writes out again. The
preview says which legs do that, because it is the difference between a
transfer bounded by the NAS and one bounded by the laptop's Wi-Fi.

**Rejected: a second engine that understands N ends.** It would re-implement
every rail above and be tested against none of the incidents that shaped them.

### A baseline is remembered per member, and it is append-only

**Chosen.** For every (sync, member, path) the sync records what that member
reported after the last run that touched it: BLAKE3 hash, size, modification
time as *that member* reported it, and the plan that wrote the row. A row may
also record that the path was absent by decision. Rows are never updated. The
current baseline is a query: the latest row per (member, path) whose plan has
not been undone.

**Why append-only.** Undo. A sync run is a journalled plan like a
reorganisation, and `undo` marks the plan `undone_at`; with an append-only
baseline that single mark also retires every baseline row the run wrote, and
the previous rows become current again with no second bookkeeping. A baseline
that was updated in place would need its own undo log.

Migration v9, additive, so every existing row stays correct:

```sql
CREATE TABLE syncs (
    id            INTEGER PRIMARY KEY,
    name          TEXT    NOT NULL,
    direction     TEXT    NOT NULL,      -- push | pull | all
    exact         INTEGER NOT NULL DEFAULT 0,
    anchor        INTEGER REFERENCES sync_members (id),  -- NULL for all
    on_conflict   TEXT    NOT NULL,      -- quarantine | rename | skip
    on_remove     TEXT    NOT NULL,      -- set-aside | delete
    verify        TEXT    NOT NULL,
    cooldown_secs INTEGER NOT NULL,
    on_launch     INTEGER NOT NULL DEFAULT 0,
    continuous    INTEGER NOT NULL DEFAULT 0,
    deleted_at    INTEGER               -- tombstone, as links have
);
CREATE TABLE sync_members (
    id         INTEGER PRIMARY KEY,
    sync_id    INTEGER NOT NULL REFERENCES syncs (id),
    ordinal    INTEGER NOT NULL,
    name       TEXT    NOT NULL,
    connection INTEGER REFERENCES connections (id),  -- NULL = this machine
    path       TEXT    NOT NULL
);
ALTER TABLE links ADD COLUMN sync_id INTEGER REFERENCES syncs (id);
                                      -- NULL = an ordinary link
CREATE TABLE sync_state (
    id          INTEGER PRIMARY KEY,
    member_id   INTEGER NOT NULL REFERENCES sync_members (id),
    path        TEXT    NOT NULL,
    hash        TEXT,                  -- NULL = absent by decision
    size        INTEGER,
    mtime       INTEGER,
    plan_id     INTEGER NOT NULL REFERENCES plans (id),
    recorded_at INTEGER NOT NULL
);
CREATE INDEX sync_state_current ON sync_state (member_id, path, id);
```

`storage::TABLES` is hand-maintained and ordered so a row only references an
earlier table; it becomes `connections, syncs, sync_members, links,
link_files, folders, plans, ops, sync_state`. A test already asserts every
table is exported; it will catch the omission if this is forgotten.

### A member is compared with itself, never with another member

**Chosen.** "Has this member changed since last time?" is answered by
comparing the member's current size and modification time with *its own*
baseline row. Modification times never cross members. A member whose backend
reports no modification time (FTP without `MLSD`, some cloud listings) is
hashed instead, and the plan says so in as many words, because it is the
difference between a free run and one that reads every file.

**Why.** Cross-member clocks are not comparable: FTP rounds to the second or
the minute, a NAS may be in another timezone, and DESIGN §9 already forbids
ordering across backends for this reason. Comparing each member with its own
past reading needs no shared clock and no preserved metadata; it only needs
the member to be consistent with itself, which every filesystem is.

Hashing is paid only where a decision needs it: when two or more members
report a change to the same path, their hashes decide whether they agree. One
changed member is delivered without reading it first; the engine hashes it
while streaming, as it always has.

### One desired content per path, decided before direction is applied

**Chosen.** The decision procedure (below) first works out, for each path,
what the set *should* hold, from what every member reports and what the
baseline remembers. Only then does the direction knob decide which members are
allowed to send and which to receive. Two steps rather than one, because the
first is the same for every mode and is the one worth property-testing.

### Nothing is removed unless the sync was told how

**Chosen.** A sync carries `on_remove = set-aside | delete`, default
`set-aside`. It governs both propagated deletions (a member deleted a file, the
set follows) and superseded copies (a member's old version, replaced by a
newer one from elsewhere). `set-aside` moves the copy into that member's
set-aside area, the one the dedup slice defines, journalled and undoable.
`delete` removes it outright, for the archive that exists to reclaim space.
Nothing else in tungstate deletes, and the flag is the one place a sync is
allowed to, so it is spelled out at `sync add` and printed when a run starts.

Without `--exact`, removal never happens at all; the modes are supersets and a
file removed by hand on one member is copied back from another on the next run.
The user chose this deliberately: a superset promise that is literally kept.

**Consequence for FTP.** A set-aside is a rename, and OpenDAL's FTP service
answers `rename` with `Unsupported`. On such a member, set-asides are refused
with the reason and every additive copy proceeds, the same rule the drain
applies to `--on-conflict replace` today, fixed by the hand-rolled FTP path
DESIGN §6 already plans. `on_remove = delete` works there, since `DELE` is not
a rename.

### A deletion through tungstate is the only deletion that sticks

**Chosen.** `tungstate sync forget <name> <path>`. In the non-exact modes a
hand deletion is repaired, which is what the superset promise means, so there
has to be a way to say *I meant it*. `forget` removes the path on every member
according to `on_remove`, records an absent-by-decision baseline row for each,
and the file does not come back. In exact modes it is a convenience; in the
others it is the only door. It is journalled as a plan and `undo` reverses it.

### An empty member is never read as a deleted member

**Chosen.** Two rails, both refusals without `--yes`. A member that lists zero
files where its baseline holds any is refused: an unmounted volume, a
mistyped root and a connection that lands in the wrong directory all look like
"everything was deleted", and an exact sync would propagate that. And a run
whose removals on any member exceed the blast limits in `core::plan::Blast`
(500 files or a fifth of the member) is refused the same way. `RootToken` is
pinned per file as the drain does, so a member that vanishes mid-run stops the
run rather than being recreated on the wrong disk.

This is the failure every cloud sync client has shipped at least once, and it
is named here so the tests can name it too.

### Conflicts are asked about, and never resolved by clock

**Chosen.** A conflict is two or more members reporting different content for
one path since the last run. `on_conflict` reuses the link vocabulary with the
same spellings and the same `wants_asking` rule:

- `quarantine` (default): in the window, each conflict is asked about, with
  apply-to-all, and the answers are *use this member's everywhere*, *keep both*
  or *leave it*. Unattended, every member keeps its own version in place and
  the other members' versions arrive in its `.tungstate-quarantine/` named after
  the member they came from, `clip (nas).mp4`; the run reports the path, and no
  baseline row is written for it, so it is asked again next time. This is
  exactly what the drain does with an arriving conflict.
- `rename`: keep both in place on every member, the copy from elsewhere named
  after its member. Never asks.
- `skip`: nothing moves for that path; it is reported until resolved by hand.

`replace` and `newer-wins` are not offered. The first has no meaning when
three members changed a file; the second would decide by comparing clocks
across members, which the design forbids and which FTP could not honour.

An edit always beats a deletion: a member that changed a file while another
deleted it is delivering, not conflicting. Deleting is cheap to redo and
editing is not.

### The decision is a pure function in `tungstate-core`

**Chosen.** `core::sync::decide(members, baseline, mode) -> SyncPlan`, with no
I/O, ordered maps only, and an `apply_to` that applies the plan on paper to
every member's snapshot, so the property tests can ask the same question the
planner's do: decide again after applying, and expect nothing. `SyncPlan`
carries the operations, the "left alone" list with reasons, a `Blast` per
member, a fingerprint per member (the planner's `fingerprint` over each
snapshot), and the legs.

```rust
pub fn decide(
    members: &[(MemberId, Snapshot)],
    baseline: &Baseline,
    mode: Mode,
) -> SyncPlan;
```

Operations are three: `Copy { to, path }`, `Remove { member, path }` and
`Record { member, path }`. `Record` writes a baseline row and nothing else; it
exists so a member that already agrees is acknowledged rather than re-read
next run.

### Copies are routed through a local member when one has the file

**Chosen.** When several members hold the desired content, the source for a
copy is a local member if there is one, then the member on the same connection
as the target, then any. Copies are grouped by (source, target) into legs, and
the plan lists the legs in the order they will run. Legs run one after another
within a run: two legs writing the same member at once would race on names,
which is the reason the window already runs a two-way exchange as sequential
links.

### `set_modified` is a defaulted trait method

**Chosen.** `Backend::set_modified(path, SystemTime) -> Result<bool>`,
defaulted to `Ok(false)`, implemented by the local backend. After a copy is
committed the leg carries the source member's modification time across where
it can, and records `false` where it cannot, so the "down to the last bit"
promise is kept where the backend allows and reported honestly where it does
not. The baseline never depends on it: each member's row stores what that
member reports afterwards, whatever that is. Six existing backend
implementations keep compiling, the same reason `read_prefix` was defaulted.

### Two runs never write one member at once

**Chosen.** Syncs sharing a member queue behind one another in the existing
`RunQueue`, and a sync run refuses to start while a drain into one of its
members is in flight. A member can belong to several syncs; that is how a
laptop folder reaches several destinations under different rules.

### Members do not overlap, and are not governed inboxes

**Chosen.** `sync add` refuses a member inside another member of the same
sync, and refuses a governed folder's inbox, for the reason DESIGN §9 refuses
nested governed folders: two controllers with one view of a directory will
fight. A member *may* be a governed folder's root, in which case the folder's
policy tidies what the sync delivers, and the sync then sees the tidied
location as a rename it must carry to the other members. That interaction is
in scope for 9c's tests and is where the two halves of the product meet.

### Continuous sync is a trigger, not a second engine (9e)

**Chosen.** While the window is open, a sync marked `continuous` watches its
local members with the slice 9 watcher and polls its remote members on the
adaptive interval DESIGN §3 describes. A change starts the cooldown; when it
elapses the same `decide` and the same legs run. The watcher's own rules apply
unchanged: self-events are swallowed, and an hourly full comparison is the
source of truth because watchers drop events. Nothing runs with the window
closed until the daemon (slice 10), which triggers the same function.

---

## The decision procedure

Per relative path, four steps. Reserved names (`.tungstate`,
`.tungstate-quarantine`, the set-aside area), the ignore list and the engine's
own temp names are excluded before step 1.

**1. Classify each member against its own baseline.**

| baseline row | member now | state |
|---|---|---|
| none | present | `new` |
| none | absent | `missing` (the member joined after the file existed) |
| content | present, same size and mtime | `unchanged` |
| content | present, size or mtime differ | `changed` (hashed only if another member also moved) |
| content | absent | `deleted` |
| absent by decision | present | `new` |
| absent by decision | absent | `unchanged` (still absent) |

A member with no modification times is classified by hash instead, and a
member inside its cooldown for this path is *left alone (too recent)* and
takes no further part.

**2. Decide the desired content.** Let C be the members `new` or `changed`, D
the `deleted`, U the `unchanged` with content, M the `missing`.

- C has one distinct hash: that content is desired. Every member not holding
  it receives it; D members receive it too, since an edit beats a deletion.
- C has two or more distinct hashes: **conflict**, handled per `on_conflict`;
  no baseline row is written for the path until it is resolved.
- C is empty and D is not: a deletion. With `--exact`, members in U have the
  path removed per `on_remove`; without, members in D receive it back from U.
  If U is also empty the path is simply gone, and absent rows are recorded.
- C and D are empty: nothing to do, except that members in M receive a copy
  from U.

**3. Apply the direction.** `--all` allows every send and receive. `--push`
allows sends only from the anchor and receives only by other members; a
non-anchor member's `new` or `changed` file is *left alone (only the anchor
sends)* without `--exact`, and removed with it. `--pull` is the mirror image:
sends only to the anchor. A `deleted` on a non-anchor member under `--push` is
repaired from the anchor in both flavours, because *every member ⊇ anchor* is
the promise.

**4. Route.** For every receive, pick the source per the routing decision, and
group into legs. The plan lists per member: files arriving, files leaving,
files removed, files left alone with their reason, bytes, and whether the
member can rename. "Files copied" and "files removed" are different numbers
and are never added together.

"Left alone" reasons reuse the planner's `Reason` and add: *too recent*, *only
the anchor sends*, *only the anchor receives*, *name clashes on a
case-insensitive member*, *needs a member that can rename*, and *conflict*.

Each run is a journalled plan: intent per file before bytes move, the file's
outcome after verify, and the baseline row only after the outcome. A crash
between the outcome and the row costs one re-comparison next run, which finds
the copies identical and records them. Never anything worse.

---

## What ships

### 9b: the set, the baseline, and the additive modes

1. **Journal**: migration v9 as above; `create_sync`, `sync_by_name`, `syncs`,
   `add_member`, `remove_member`, `remove_sync` (refused while a leg has
   `intended` rows, retired otherwise), `baseline_for(sync)` (the latest-row
   query), `record_baseline(rows, plan)`; `links.sync_id` honoured by `links()`
   so legs are hidden; `storage::TABLES` extended.
2. **`tungstate-core`**: `sync.rs` with `decide`, `SyncPlan`, `apply_to`, the
   reasons, per-member `Blast`, legs and routing. `--push`, `--pull`, `--all`
   without `--exact`; `Remove` is never emitted in this sub-slice.
3. **`tungstate-backend`**: `set_modified`, defaulted; local implementation.
4. **`tungstate-transfer`**: a leg runner that takes a `SyncPlan`, creates or
   reuses the owned links, runs each leg through `run_selection` with `Keep`,
   carries modification times where `set_modified` allows, and returns per-leg
   summaries. Baseline rows are written by the caller after each leg's
   outcomes, never inside the engine.
5. **CLI**: `tungstate sync add <name> <end>... [--push|--pull|--all]
   [--anchor] [--as ...] [--on-conflict] [--on-remove] [--verify] [--cooldown]`,
   `sync list`, `sync members <name> add|remove <end>`, `sync preview <name>`
   (the plan, per member, changing nothing), `sync run <name> [--yes]
   [--parallel]`, `sync remove <name>`; every one takes `--json`. `--exact`
   and `--on-remove delete` are accepted and refused with "9c" until then, so
   the flags are stable from the first release.

### 9c: exact, removal, conflicts, forget and undo

1. `--exact` in `decide`; `Remove` emitted and carried out per `on_remove`,
   through the dedup slice's set-aside area, or `remove_file` for `delete`.
2. The empty-member rail and the per-member blast refusal, in the CLI beside
   the reorganisation breaker, both `--yes`-able.
3. Conflicts: `quarantine` unattended (parked copies named after their
   member), `rename`, `skip`; `sync resolve <name> <path> --keep <member> |
   --keep-both` for the command line.
4. `sync forget <name> <path>`.
5. `undo` extended: a sync run is a plan; its inverse sets aside what it
   delivered and restores what it removed, in reverse, checked not forced, and
   restores the previous baseline by the append-only rule. `invert` gains
   `Copy → Remove` and `Remove → Restore` for rows a sync wrote.
6. The governed-member interaction: a member that is also a governed folder,
   tidied between runs, is seen as renames and carried across.

### 9d: the window

1. A **Sync** tab: syncs as rows, members as a row of places, the mode in plain
   words chosen by the same method 7b used, and the anchor marked.
2. A preview per member before the button: arriving, leaving, removed, left
   alone with reasons, bytes, legs that read through this machine. Counts
   cross the seam as data (`SyncPreviewView`), never as sentences.
3. Run, **Stop after this file** and **Stop now**, the existing progress rows
   per leg, conflict asking with apply-to-all through the existing conflict
   channel, and the way back on the same screen: *Put it back* on the last
   run.
4. `on_launch`: syncs so marked run after the interrupted-work check when the
   window opens, one after another, with the preview shown and a single
   confirmation unless the sync is also marked to run without asking.
5. `SEAM.md` gains the sync commands and their return types.

### 9e: continuous while the window is open

1. `continuous` on a sync; local members watched, remote members polled on the
   adaptive interval; a change arms the cooldown and a run follows.
2. Self-event suppression for the legs' own writes, and the hourly full
   comparison as the source of truth.
3. A status per member in the tab: watching, polling every N, paused
   (unreachable), last run.

---

## Acceptance criteria

Every sub-slice: `cargo fmt --all --check`, `cargo clippy --all-targets
--locked -- -D warnings` and `cargo test --locked` clean; CI green on Linux,
macOS and Windows; `npm run build` for 9d and 9e.

**9b**

- Three local members, `--all`: a run delivers every file to every member; a
  second run does nothing and says so; a file edited on one member is delivered
  to the other two and nothing else moves.
- `--push` with the anchor: a file added on a non-anchor member is reported as
  left alone with the reason, and a file deleted on a non-anchor member comes
  back.
- `--pull`: the mirror of the above.
- A member added to an existing sync is filled on the next run, from a local
  member when one holds the file.
- A member whose backend reports no modification times is hashed, and the
  preview says so.
- Over FTP (the Docker suite on Linux CI): a leg to and from the FTP member,
  killed mid-file, resumes on the next run and sweeps its orphans.
- **By hand:** the CapCut scenario between the laptop and the NAS mount, then
  again with the FTP connection as a third member.

**9c**

- `--all --exact`: a hand deletion on one member is set aside on the others;
  with `--on-remove delete` it is deleted; both are journalled and `undo`
  restores every member and the previous baseline.
- A member that lists nothing where its baseline has files is refused without
  `--yes`, naming the member. Removals past the blast limit are refused the
  same way.
- The same file edited on two members: unattended, each keeps its own and the
  other's copy is parked beside it named after its member; the run reports the
  path; the next run asks again. `rename` and `skip` behave as specified.
- An edit on one member and a deletion on another: the edit is delivered.
- `sync forget` removes on every member and the file does not return in a
  non-exact sync.
- A set-aside on a member that cannot rename is refused with the reason while
  the additive copies of the same run proceed.
- A governed member tidied between runs: the tidy is carried to the other
  members as renames and no file is copied twice.
- **By hand:** delete a project on the laptop, run, find it in the NAS's
  set-aside area, undo, find it back.

**9d**

- Every state looked at, not inferred, with the capture method in memory: an
  empty tab, a sync with two and with four members, a preview with removals, a
  conflict prompt, a refused empty member, and the run-on-launch confirmation.
- The five safety properties in `SEAM.md` hold on the new screen, in
  particular that "files copied" and "files removed" are separate numbers and
  "left alone" keeps its reasons.
- **By hand:** a day of use of the CapCut sync with no terminal open.

**9e**

- Save a file into a watched local member and see it arrive on another member
  after the cooldown, with no button; save it again inside the cooldown and see
  the timer restart rather than a double delivery.
- The legs' own writes do not trigger a second run.
- A remote member that becomes unreachable shows as paused and resumes on its
  own, with no error per poll.
- **By hand:** leave the window open through an afternoon of exports.

---

## Tests

**Property tests** (`tungstate-core`, over two to four generated members with
random files, random baselines and random modes):

- deciding again after `apply_to` yields no operations, in every mode;
- the multiset of hashes across all members plus every set-aside area is the
  same before and after, whatever `on_remove` says short of `delete`;
- after apply, `--all --exact` leaves every member equal; `--all` leaves every
  member equal to the union; `--push` leaves every member a superset of the
  anchor; `--pull` leaves the anchor a superset of every member; the `--exact`
  one-way forms leave the promised equalities;
- the plan is byte-identical across two runs of the same input;
- no operation targets a reserved name or a path outside a member's root.

**Unit tests** (`tungstate-core`): every row of the classification table;
every branch of step 2; the direction filter of step 3 for all three
directions, both flavours; routing preferring a local source; a member with no
modification times classified by hash; an edit beating a deletion; a
conflict with three distinct hashes.

**Unit tests** (journal, execute, transfer): the latest-row baseline query
before and after an undo; `record_baseline` refusing rows for an undone plan;
`invert` of a sync's rows; the leg runner writing no baseline row when a file
fails verify; `set_modified` carried and recorded false where unsupported;
`links()` hiding legs.

**CLI tests**: the full round trip through the binary with three members
under `sandboxed()`; the empty-member refusal and the blast refusal with their
messages; `--exact` refused with "9c" in 9b; `forget` and `undo`; `--json`
snapshots for `sync preview` and `sync list`.

**Snapshots** (`insta`): the `--help` texts for the `sync` noun, and one
`sync preview` rendering with every kind of row present.

**Known breakage:** the storage export test that enumerates tables gains four
names, deliberately.

---

## Out of scope

- Running unattended with the window closed: the daemon, slice 10, which
  triggers the same function.
- Native verification and exact byte-offset resume between two tungstate
  nodes: the peer protocol, slice 10b. A `tungstate://` member then works
  unchanged here.
- SMB, WebDAV, S3 and cloud members: they arrive as connection schemes in
  slice 12 and are members the moment they exist, with no change to this
  slice. FTP is already here.
- Syncing a selection of files rather than a whole member: the drain's
  `link_files` idea applied to a sync, slice 15+.
- Version history beyond one set-aside copy per removal, and a cross-member
  content index for "is this already somewhere in the set": slice 15+, with
  cross-folder dedup.
- `newer-wins` and `replace` for conflicts: decided above, not deferred.
- Renaming a member: as with connections, remove and re-add; the baseline rows
  belong to the member id and a new member is filled on the next run.
