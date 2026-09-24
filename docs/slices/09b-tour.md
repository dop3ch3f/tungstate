# Slice 9b tour: a set of folders kept in step

`tungstate sync add capcut ~/CapCut /Volumes/nas/capcut --all`, then
`tungstate sync run capcut`. The laptop's new exports go to the NAS, the NAS's
archive comes to the laptop, and a second run moves nothing.

Brief: [09b-folder-sync.md](09b-folder-sync.md), which covers 9b to 9e. This
tour covers 9b: the set, the baseline, and the three modes that only ever add.

---

## 1. The decision is a pure function over N members

`tungstate_core::sync::decide` takes every member's listing, the baseline and
the mode, and returns a plan. It reads no files and touches no disk:

```rust
pub fn decide(members: &[Member], baseline: &Baseline, digests: &Digests, mode: &Mode) -> SyncPlan
```

Each path goes through four steps, in `decide_path`:

1. **Each member against its own baseline.** New, missing, unchanged, changed,
   deleted, or gone. Size and modification time against *that member's own*
   last reading. Clocks never cross members, because FTP rounds to the minute
   and a NAS keeps its own timezone.
2. **Who may send and who may receive**, from the direction. Under `--push`
   only the anchor sends. Under `--pull` only the anchor receives.
3. **What the set should hold:** what moved on a sender, or failing that what a
   sender still holds unchanged.
4. **Who needs it**, and where each copy reads from: a member on this machine
   first, then one on the target's own connection, then any.

Pure has a practical payoff: `apply_to` plays a plan out on paper, and the
property tests decide again and expect nothing.

## 2. When bytes are needed, the decision asks

A pure function cannot read a file, and sometimes a decision needs one: two
members both changed a path and it matters whether they agree, or a member
keeps no modification times. So `decide` returns **questions** alongside the
plan, and those paths get no operations yet:

```rust
pub struct Question { pub member: MemberId, pub path: String, pub sampled: bool, pub because: Because }
```

`tungstate_sync::decide` answers them, using the duplicate finder's own
`Cached` digester, which already reads over any backend and remembers what it
read, and asks again. Two rounds settle everything; the loop stops at four so a
bug cannot become a hang.

The common case asks nothing. Two members that have not moved since they were
recorded are compared by what was recorded. The first draft read them again,
which on the second run of an archive would have reread all of it.

## 3. How hard to look the first time is the person's choice

When two members already hold a path the sync has never seen, only reading can
say whether they match. You asked for all three levels to be offered, with the
cost said plainly, so `--first-check` is a setting on the sync:

| | reads | risk |
|---|---|---|
| `full` (default) | both copies, in full | none |
| `sampled` | the start, middle and end | a difference only between samples is missed |
| `size` | nothing | an edit that kept the size is missed |

It only ever applies to that first meeting. After that, each member is compared
with itself, which reads nothing.

A sampled digest carries a `sampled:` prefix and is **never remembered**. It
differs from the whole digest of the same bytes, so a remembered one would meet
a whole one on a later run and call two identical files different. A test
caught the first version of this looping forever: hiding the digest from the
record also hid it from the comparison, so it asked for the same read again.

## 4. The baseline is append-only, and "current" is a query

The journal gains four tables in v12: `syncs`, `sync_members`, `sync_state`,
and `links.sync_id`. `sync_state` is only ever inserted into. The current
reading of a path is the latest row whose plan has not been undone:

```sql
AND s.id = (SELECT MAX(t.id) FROM sync_state t JOIN plans q ON q.id = t.plan_id
            WHERE t.member_id = s.member_id AND t.path = s.path AND q.undone_at IS NULL)
```

That is what will make 9c's undo cheap. Marking a run's plan undone retires
every row it wrote, and the rows before them become current again, with no
second log to keep in step. There is a test for exactly that already.

## 5. A leg is a link, and the engine is unchanged

Bytes move between two members at a time, through `tungstate-transfer`, as a
link the sync owns (`sync:<sync>:<from>><to>`). Everything the drain has
earned comes with it: temp names, verify before commit, resume after a crash.
The Links tab never shows them, because they are not saved pairs.

Every leg runs with `ConflictAction::Replace`, and on purpose. A version the
target already holds at that path is one the decision has chosen to
supersede, and `Replace` is the engine's "set the old one aside, then write".
An edit arriving never costs the version it replaces; the by-hand run found
the laptop's old `photo1.jpg` in `.tungstate-quarantine/` afterwards.

Adding the link was done beside `create_link`, not by giving `NewLink` a new
field: half a dozen callers build `NewLink` by hand and none of them is a
sync. `create_leg` sets the one extra column.

## 6. `set_modified` is a trait method with a default

```rust
fn set_modified(&self, path: &Path, at: SystemTime) -> Result<bool> {
    let _ = (path, at);
    Ok(false)
}
```

A trait method with a body is a default: every backend gets it for free, and
only the ones that can do better override it. `LocalBackend` does; the six
others keep compiling untouched. `Ok(false)` means "cannot here", which is not
an error. A sync carries the time across where it can and never depends on
it, since each member is only compared with its own reading.

## 7. Remember a path only once every copy of it landed

`tungstate-sync` is a new crate, one level up from `tungstate-watch`: it opens
the members, calls the decision, runs the legs and writes what landed. It
exists so the command line now, and the window in 9d, call one function.

One rule in it is easy to get wrong. If the source's new version were
remembered while a copy of it failed, the next run would see two members that
each look unchanged and hold different bytes, and call it a conflict instead of
finishing the copy. So a path's readings are written only when every copy of
it landed. A failure is simply decided again next run.

## 8. The command line

```
tungstate sync add <name> <folder> <folder>... --push | --pull | --all
                  [--anchor] [--as NAME]... [--first-check] [--cooldown] [--verify]
tungstate sync list | preview <name> | run <name> [--yes] [--parallel N]
tungstate sync members <name> add <folder> [--as] | remove <member>
tungstate sync remove <name>
```

The direction has to be said. `--all` brings a NAS's whole archive to a
laptop, which is not something to do by leaving a flag out. `--exact` and
`--on-remove delete` are accepted by the parser and refused with "9c", so the
flags do not change when 9c lands. Every listing takes `--json`.

A member is named after its connection (`nas:capcut` is `nas`) or its folder,
and `--as` overrides it. The brief called every local member `here`, which
collides the moment two local folders share a sync.

## 9. Where this departs from the brief

- **Migration v12, not v9**: v9 to v11 were taken by the time this landed.
- **`sync_state.present`** rather than "a NULL hash means absent". With
  `--first-check size` a present file can have no digest at all.
- **`syncs.anchor` has no foreign key.** Members point at their sync, and a
  cycle of foreign keys cannot be imported one table at a time.
- **`first_check`**, a column the brief did not have, per your answer.
- **A sync run is a plan of purpose `sync`**, beside tidy and duplicates.
- **`tungstate-sync` is its own crate**, where the brief had the CLI do the
  orchestrating. The window needs the same function.

## 10. What checking found

- **By hand, against the NAS over SMB:** a file identical on both sides was
  recorded as agreeing and *also* listed as left alone ("changed on nas, and
  only the anchor sends"). The report was made before agreement was known.
  Fixed, with a test, and the wording now says "nas has it, but only the
  anchor sends", which is true of a file the sync is meeting for the first time.
- **A property test** produced a history where a file had the size and time
  it was recorded with but different bytes. Size-and-time checking cannot see
  that by design, and the brief chose it knowingly, so the generator now never
  builds such a history, with a comment saying why.
- **A mutation check** (breaking the code on purpose) failed three tests, so
  the properties bite.

The by-hand run, on a test folder on the NAS: laptop and NAS each gained what
the other had, byte-identical; a second run did nothing; an edit on the NAS
reached the laptop with the old version set aside; a third member was filled
from the laptop rather than over the network; `--first-check sampled` sampled.
The test folder was removed and the share's top level matched before and after.

## 11. What is checked

- **19 in `tungstate_core::sync`**, three of them properties over 2 to 4
  members, random files, random histories and every mode: deciding again after
  a run does nothing, each direction keeps its promise, and the same input
  gives the same plan. Run at 20,000 cases each while building.
- **6 in `tungstate-journal`**, including the baseline before and after an
  undo, and legs staying out of the saved pairs.
- **6 in `tungstate-sync`**, against real folders with the real engine.
- **4 in the CLI**, including the whole round trip through the binary and
  both help texts as snapshots.
- 603 tests in parallel and again one at a time, `clippy -D warnings`,
  `fmt --check`, `npm run build`, and CI on Linux, macOS and Windows.

## What is still not verified

- **No FTP member.** A leg is a link, and links over FTP are tested in CI's
  Docker suite, but no sync has had an FTP member, and the brief's "killed
  mid-file over FTP, resumes" test was not written for syncs.
- **Two syncs sharing a member are not queued.** The brief has them wait in
  the window's `RunQueue`; the command line runs one at a time by hand, and 9d
  is where the queue arrives.
- **A member inside a governed folder's inbox is not refused.** Members that
  overlap each other are.
- **Nothing large.** The biggest by-hand run was six photographs.
