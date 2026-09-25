# Slice 9c tour: deletions, conflicts, and the way back

`tungstate sync add capcut ~/CapCut /Volumes/nas/capcut --all --exact`. Delete
a project on the laptop, run, and the NAS copy is in the NAS's set-aside area.
`tungstate sync undo capcut` puts it back on the NAS, and the next run puts it
back on the laptop too.

Brief: [09b-folder-sync.md](09b-folder-sync.md), section 9c. The 9b tour is
[09b-tour.md](09b-tour.md). This tour covers what a sync now takes away, what
it does when two members disagree, and how a run is taken back.

---

## 1. `--exact` is one more answer to "what should the set hold?"

Step 3 of the decision works out the content every member should hold. In 9b
the answer was always a file: what a sender changed, or failing that what a
sender still held. `--exact` adds a third answer, *nothing*:

```rust
let gone_everywhere = changed_senders.is_empty()
    && mode.exact
    && (unchanged_senders.is_empty() || (mode.direction == Direction::All && deleted));
```

Read it as: nobody changed the file, and either no sender holds it (under
`--push` the anchor deleted it or never had it; under `--pull` no other member
has it), or this is `--all` and somebody deleted it. Then `remove_everywhere`
takes it off every member that receives. An edit still beats a deletion: if
anyone changed the file, `changed_senders` is not empty and the file is
delivered, not removed.

Under `--exact` a member that may not send and changed a file anyway is no
longer "left alone, only the anchor sends". It is replaced. That is the
promise: under `--push --exact`, every member is exactly the anchor.

## 2. Taking a copy off is data, and it is journalled like a rename

```rust
pub enum Removal {
    SetAside { to: String },
    Delete,
}
```

A Rust `enum` variant can carry its own data, and `SetAside` carries the
name it will be set aside under. The name is chosen when the run is
*decided*, not when it runs, so the preview and the run agree about it and
the property tests can check it.

The runner journals a set-aside as an `OpKind::Rename` from the file to its
set-aside name. Undo already knows how to reverse a rename, so a set-aside
needed no new kind of op. Only an outright delete is an `OpKind::Remove`, and
a run that has one is written with `reversible = false` **before its first
op**, so a crash half-way cannot leave a record that claims it can be taken
back.

Before taking anything off, the runner checks the file is still the size and
time it had when the run was decided. Someone may be writing it right now.

## 3. The old version is set aside by the sync, not the engine

In 9b a leg ran with `ConflictAction::Replace`, and the engine moved the old
version aside before writing the new one. Mapping 9c showed two problems with
that. The move was not journalled, so undo could not find it. And it always
used the same name, so a second edit of the same file overwrote the first
parked version.

Now the decision emits a `Remove { because: Superseded }` before every copy
onto a file a member already holds. The runner does all removals and renames
first, journalled, then runs the legs with `ConflictAction::Skip`. Anything
the engine still finds in the way arrived after the run was decided, and is
not this run's to replace. The engine's `Replace` is unchanged, so the drain
behaves exactly as before.

A second edit of `a.mp4` now parks as `a-2.mp4`. The numbering is dupes'
`free_name`, shared through `free_name_by`, which takes a closure for "is this
name taken?":

```rust
pub fn free_name_by(wanted: &str, taken: impl Fn(&str) -> bool) -> String
```

`impl Fn(&str) -> bool` in argument position means "any closure that takes a
`&str` and returns a `bool`". Dupes passes one that asks a snapshot; the sync
passes one that asks every member at once.

## 4. Conflicts: park, retire the name, or wait

When members changed a file differently, `on_conflict` decides:

- **quarantine** (default): each member keeps its own version, and receives
  the others' in its set-aside area as `clip (nas).mp4`. Nothing is
  remembered for the path, so the next run asks again.
- **rename**: the contested name is retired. Every member ends up with
  `clip (laptop).mp4` and `clip (nas).mp4`, and `clip.mp4` goes away. You
  chose this over keeping the original name, because it settles in one run.
- **skip**: nothing moves until `tungstate sync resolve`.

Asking again every run must not park again every run. Before parking, the
decision looks in the target's set-aside area for `clip (nas).mp4` or a
numbered `clip (nas)-2.mp4` of the same size, and compares digests. A version
already there is not sent twice. That is a new question the decision can ask,
`Because::Parked`, and `Member` gained `parked`, a listing of its set-aside
area.

Parking has to write `clip (nas).mp4` directly, because an FTP member cannot
rename afterwards. So the engine gained one additive builder method:

```rust
#[must_use]
pub fn landing(mut self, names: BTreeMap<PathBuf, PathBuf>) -> Self {
    self.landing = names;
    self
}
```

`mut self` takes the `Transfer` by value, changes it, and hands it back. That
is the builder pattern Rust uses where other languages would return `this`.
`#[must_use]` makes the compiler warn if someone calls it and drops the
result, which would silently throw the setting away.

`tungstate sync resolve capcut clip.mp4 --keep nas` puts one version
everywhere and sets the others aside; `--keep-both` retires the name whatever
`on_conflict` says. Both arrive in the decision as an `Asked`:

```rust
let asked = Asked {
    resolve: [(path.clone(), resolution)].into(),
    only: true,
    ..Asked::default()
};
```

`..Asked::default()` is struct update syntax: "every field I did not name
comes from this value". `[(k, v)].into()` builds a `BTreeMap` from an array,
because `BTreeMap` implements `From<[(K, V); N]>`. `only: true` limits the
decision to the named paths, so resolving one file does not run the whole
sync.

## 5. `sync forget` is all or nothing

Without `--exact`, a file deleted by hand is copied back; `forget` is how to
say you meant it. It takes the path, or everything under a folder, off every
member and remembers it as absent everywhere, so no run brings it back.

If any member holding it cannot set it aside (FTP, with `on_remove =
set-aside`), the whole forget is refused and nothing moves. A forget that
reached some members and not others would be copied back to them by the next
run.

## 6. Undo is checked on paper first, across every member

`tungstate sync undo capcut` puts back the newest run; `--last N` and `--plan
ID` pick others. `tungstate undo --plan ID` notices the plan is a sync run
before it looks for a folder, and routes to the same code.

`tungstate_execute::invert_sync` is pure: committed ops in reverse, a copy
becomes "take it off again", a rename becomes "move it back", and a delete
makes the whole run irreversible. `tungstate_sync::undo` then checks every
step before doing any of them:

- a copy is only taken off if it still holds the bytes that were delivered;
- a file only moves back to a name nothing has taken since;
- a step already done counts as done, so a half-finished undo can be run
  again.

The checking runs on a paper model: a `BTreeMap<(member, path), bool>` of
what each step will have changed, falling back to asking the member. The
closure that asks takes the map as an argument instead of capturing it:

```rust
let exists = |paper: &BTreeMap<(usize, String), bool>, place: usize, path: &str| { ... };
```

A closure
that captured `paper` would borrow it for as long as the closure lives, and
the loop also needs to *insert* into `paper`. Rust allows many readers or one
writer, never both at once, so capturing it would not compile. Passing it in
borrows it only for the length of each call.

Runs come back newest first. A later run's readings would still be current
over the undone one's, and would make the files put back look like edits.

**The deletion that caused a run is undone too.** You chose this. The member
you deleted on gets an "absent" reading under the undo, so it becomes an
ordinary member that lacks the file, and the next run copies it back to
that member instead of carrying the deletion out again.

The baseline needed nothing new. It is append-only (9b, section 4), so
marking the run undone retires every reading it wrote.

## 7. A governed member's tidy is carried as a rename

A folder with a policy tidies `clip.mp4` into `2026/clip.mp4`. Read as a
deletion plus a new file, an exact sync would set the old name aside
everywhere and copy the new one. A non-exact sync would copy the old name
straight back, and the sync and the policy would fight forever.

`detect_moves` runs before anything else. On each member that sends, it pairs
a file that vanished with one that appeared at the same size, and reads the
new one to compare digests. A match, where no other member has touched either
name, is carried as a rename to every receiver: a real rename where the member
can, a copy then a delete of the old name where it cannot and `on_remove` is
`delete`, and left alone with the reason where it would need a set-aside it
cannot do.

The vanished files are grouped by size first:

```rust
let mut gone: BTreeMap<u64, Vec<(&String, Seen)>> = BTreeMap::new();
```

so a tidy of ten thousand files looks each candidate up by size. The first
draft compared every new file with every vanished one, which is a hundred
million comparisons for that tidy. The sender of a file never read it, so its
recorded digest is often missing. The digest the copy computed on the
receiving member stands in, and runs now also remember that digest against
the sender.

A sync with `--first-check size` records no digests, so it cannot tell a move
from a delete and an add, and treats them that way.

## 8. The refusals are data, and the command line words them

```rust
pub enum Refusal {
    Hollow { member: String, held: usize },
    Blast { member: String, taking_off: usize, of: usize },
}
```

`tungstate_sync::refusals` says what needs a person's yes; the command line
turns it into the same kind of sentence the reorganisation breaker uses, and
9d's window will word it its own way. Neither can drift from what is checked.

- **Hollow**: a member lists no files where after the last run it held some.
  An unmounted share, a mistyped folder and an FTP login that lands in the
  wrong place all look exactly like "everything was deleted".
- **Blast**: more than 500 files, or a fifth of a member, taken off in one
  run. Replaced versions count, as you asked: a NAS restored from an old
  backup looks like every file edited.

Separately, a run that takes anything off a member and has nobody at the
terminal to confirm it is refused without `--yes`. In 9b an unattended run
went ahead because a sync never removed anything, and that is no longer true.

## 9. `sync set`

`tungstate sync set capcut --exact --on-remove delete` changes a sync from its
next run on, without losing what it remembers. The members and the direction
cannot change this way: changing either would make the baseline describe a
different sync. `add` and `set` share one function, `settings()`, so they
cannot disagree about what is allowed; `--on-remove delete` needs `--exact`
in both.

A leg's link keeps the verify level it was made with. The runner now
overrides it with the sync's current one, so `sync set --verify` reaches legs
that already exist.

## 10. Where this departs from the brief

- **`sync undo <name>`, not `undo --last 1`.** The top-level `undo` works on
  a folder; a sync run spans several. `undo --plan ID` still works for a
  sync run.
- **The sync sets old versions aside, not the engine**, and legs use `Skip`.
- **`rename` retires the name** rather than keeping the original in place.
- **Undo also undoes the deletion that caused the run.**
- **No journal migration.** Everything 9c needs was already in v12:
  "already parked" is recognised by digest, not remembered.
- **`Transfer::landing`**, the one addition to the engine.
- **`sync set`**, which the brief did not have.
- **Folders a removal empties are removed too**, on the member the file was
  taken off and in the set-aside area after an undo. A folder is not
  something a sync keeps in step, so without this an empty `project/` stayed
  on the NAS after the project was deleted on the laptop.

## 11. What checking found

- **Four property-test failures, each a real bug**, found at 20,000 to
  300,000 cases:
  - under `--pull`, two members could send different versions to the anchor
    while only one had changed, and the next run called it a conflict. The
    unchanged sender is now compared in the same run;
  - with `--first-check size`, digests read for another reason (move
    detection) changed the verdict on one run and not the next. Size only now
    means size only, and records no digest;
  - "first meeting" was decided for the whole set, so a copy that landed
    elsewhere in a run that could not finish changed how two other members
    were compared. It is now asked of the pair;
  - move detection asked for one read per round rather than all of them at
    once, and a big tidy ran out of rounds.
- **A 9b bug over FTP.** Reading a file's first 64 KiB fails in OpenDAL's FTP
  service when the file is shorter than that, with "reader got too little
  data" rather than the "range not satisfiable" the adapter expected. Every
  sync with a small file on an FTP member failed on its second run. The
  adapter now checks the file's size, but only after such a failure, so a
  file longer than 64 KiB still costs one round trip. The new FTP sync tests
  fail without the fix.
- **By hand**: the empty folder left behind (section 10).
- **A review of the diff** found two quadratic loops (section 7, and the
  runner's last phase searching backwards through every op) and fixed both.

## 12. By hand, against the NAS over SMB

In one dated test folder on the share, with a throwaway journal:

- nine files synced laptop to NAS; the second run did nothing;
- the project deleted on the laptop: set aside on the NAS; a run with nobody
  to confirm was refused;
- `sync undo`: back on the NAS, and the next run brought it back to the
  laptop, byte for byte;
- the same file edited on both: each side parked the other's as
  `export1 (nas).mp4` and `export1 (laptop).mp4`; the second run parked
  nothing new; `sync resolve --keep nas` settled it;
- `sync forget capcut project`: gone from both, not back after a non-exact
  run, back again after `sync undo`;
- a policy on the laptop tidied every `.mp4` into `Video/`: carried to the NAS
  as 9 renames, nothing copied;
- the share unmounted mid-way: the run stopped with nothing touched.

The test folder was removed, the share's top level matched the listing taken
before, and the share was unmounted again as it was found.

## 13. What is checked

- **46 in `tungstate_core::sync`**, 5 of them properties over 2 to 4 members,
  random files and histories, every direction, both `exact` settings, all
  three conflict settings and members that cannot rename: deciding again after
  a run does nothing, each direction keeps its promise (exactly, under
  `--exact`), no content is lost unless the sync deletes, only a set-aside or
  a park writes into the set-aside area, and the same input gives the same
  plan. Run at 300,000 cases each while building.
- **16 in `tungstate-sync`** against real folders and the real engine,
  including an undo across members, a member that cannot rename (a wrapper
  backend), a conflict parked twice, and a file edited after the run was
  decided.
- **4 in `tungstate-execute`**, **3 in `tungstate-journal`**, **1 in
  `tungstate-transfer`**.
- **9 new or rewritten in the CLI**, including undo by plan id from outside any folder, a
  governed member tidied by `apply`, and one preview rendered with every kind
  of row as a snapshot, plus the four new `--help` texts.
- **2 new over FTP**, run locally against the Docker server as well as in CI.
- 656 tests in parallel and again one at a time, `clippy -D warnings` and
  `fmt --check`.

## What is still not verified

- **The window.** Nothing here is in it yet; 9d.
- **A member inside a governed folder's inbox is still not refused.** A member
  that *is* a governed folder's root works, and is tested.
- **Two syncs sharing a member are not queued against each other.** 9d.
- **An FTP leg killed mid-file** has still not been tried for a sync (the
  drain's test covers the engine).
- **`--on-conflict rename` and `--keep-both`** are tested on paper and on real
  folders, not by hand on the NAS.
- **Nothing large.** The biggest by-hand run was about 2 MB.
