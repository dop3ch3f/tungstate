# Slice 9: the folder keeps itself in order

**Goal:** a governed folder notices what arrives in it and files it once the
file has stopped changing, without anybody pressing a button.

**Runnable outcome:** `tungstate watch` in a terminal, and the window doing the
same while it is open. Save a download into a governed folder, wait, and find
it where the rules say it belongs.

This is the last slice before folder sync ([9b–9e](09b-folder-sync.md)), and
9e is the same machinery with a different trigger.

---

## What is already here, and it is more than it looks

- **The settle rule.** `[defaults].cooldown` has been in the policy since
  slice 5, and `Plan::wanted` already holds back any file written more
  recently than that with `Reason::TooRecent`. *Wait until it stops changing*
  is written, shipped and tested. The watcher does not implement it; it comes
  back later and asks again.
- **The modes.** `observe`, `suggest`, `enforce` already mean report, propose
  and apply.
- **The whole pass.** Survey, plan, apply, journal, undo. A watcher-driven tidy
  is the same plan through the same executor, so it lands in history and comes
  back with `undo` exactly like one somebody pressed.
- **Network detection.** `on_a_network_volume` arrived in 8b and is the reason
  a share can be told apart from a disk.

## What is missing

- Nothing listens to the filesystem.
- Nothing runs on a timer.
- Nothing swallows the app's own writes, so every move it made would come back
  as an event about a file that just changed.

---

## Decisions taken (2026-09-23)

### 1. It moves files only in `enforce`

**Asked and answered by the user.** A folder set to `enforce` is tidied on its
own, which is what that word already means everywhere else. A folder set to
`observe` or `suggest` is **never touched**: the watcher notices, counts, and
says so, and the move still waits for a person.

That makes the visible outcome different in the two cases, and both are worth
having. In `enforce` the folder stays in shape. In the other two you get
*4 files arrived in Downloads since you last tidied*, which is the thing that
makes somebody go and look.

### 2. Every governed folder is watched

**Asked and answered.** No per-folder switch to forget to turn on. What happens
when something arrives is already decided by that folder's own mode, so the
second knob would only be a way to make the first one lie.

One switch exists, and it is global: watching can be turned off entirely.

### 3. The watcher is an optimisation, never the truth

Watchers drop events. inotify overflows its queue, FSEvents coalesces, and a
file that arrives while the machine is asleep may generate nothing at all.

So a **periodic rescan runs regardless**, hourly by default, and it is the
thing that is actually correct. The watcher exists to make the common case
fast, not to be believed. Any bug in the event handling costs a delay of up to
an hour, not a file left in the wrong place for ever.

### 4. The app's own writes are swallowed

Every move tungstate makes echoes back as an event about a file that just
changed, which marks the folder dirty again and buys another survey.

A tidy converges, so the second pass finds nothing and the loop ends. That
makes this a **cost problem rather than a correctness one**, and the cost is
one full survey of the folder per action, which on a large folder is the
difference between usable and not. Paths the executor wrote are remembered for
a few seconds and matching events are dropped.

### 5. A share is polled, not watched

DESIGN §6 already says a network mount looks local and is not, and that its
capabilities are downgraded including *no watcher trust*. FSEvents and inotify
report nothing about a change another machine made to an SMB share.

So a governed folder on a share is not watched at all. It gets the periodic
rescan and nothing else, and the window says which of the two it is doing,
because "watching" and "checking every hour" are different promises.

### 6. Synchronous, one thread

`notify` is sync-native, which is why DESIGN §7 puts the daemon's async at
slice 10 and not here. One thread owns the watcher, a standard channel carries
events, and the loop blocks on a receive with a deadline. No `tokio`.

### 7. It stops when the app stops, and says so

There is no daemon until slice 10. `tungstate watch` runs until Ctrl-C, and the
window watches while it is open. Anything that implies otherwise would be a
promise this slice cannot keep, so the window says *while Tungstate is open*
next to the switch rather than after somebody notices.

---

## The shape

A new crate, `tungstate-watch`, holding the loop and nothing else.

```rust
pub enum Noticed {
    /// Files arrived and settled; this is what was done about them.
    Tidied { folder: String, files: usize, plan: i64 },
    /// Files arrived in a folder that does not move things on its own.
    Waiting { folder: String, files: usize },
    /// The hourly pass ran.
    Swept { folder: String, files: usize },
    /// A folder could not be read, or its rules will not load.
    Trouble { folder: String, why: String },
}

pub fn watch(
    folders: &[Watched],
    journal: &Journal,
    stop: &Stop,
    mut told: impl FnMut(&Noticed) + Send,
) -> Result<(), WatchError>;
```

The callback is the same seam the duplicate scan uses: the command line prints,
the window emits an event, a test counts.

### The loop, in order

1. Block on the debounced event channel with a deadline, which is the earliest
   of every folder's pending action and the next hourly sweep.
2. An event maps to the governed root that contains it. Anything inside
   `.tungstate/` or the set-aside area is ignored, and anything in the expected
   set is dropped.
3. That root is marked pending at `now + cooldown`.
4. When a pending deadline passes, the folder is surveyed and planned. In
   `enforce` the plan is applied and every path it wrote goes into the expected
   set; otherwise the count is reported and nothing moves.
5. On the hourly deadline, every folder is swept whether or not anything was
   noticed.

## What this slice must never do

1. **Never move anything in a folder that is not `enforce`.**
2. **Never move a file that is still changing.** The cooldown is the planner's,
   unchanged, so a watcher-driven tidy and a hand-driven one hold files back
   for exactly the same reasons.
3. **Never be the only thing that looks.** The sweep runs even if no event ever
   arrives.
4. **Never move something invisibly.** Every action is journalled, appears in
   History, and comes back with `undo` like any other.
5. **Never claim to be running when it is not.** No daemon until slice 10.

## The command line

`tungstate watch [PATH...]`, watching the folders named or every governed one,
printing a line per thing noticed and running until Ctrl-C. `--sweep <DUR>`
changes the hourly pass; `--once` does a single sweep and exits, which is what
a `cron` line would want and what the tests use.

## The window

- A switch in Settings: *keep folders in order while Tungstate is open*, and
  the sentence that says exactly that.
- On Home, what the watcher has done since the window opened, as a short list.
  This is the first thing in the app that happens without being asked, so it
  says what it did rather than leaving it to History.
- On the folder screen, a line when files have arrived and the folder is not
  set to move things on its own: *4 files arrived. Tidy up to file them.*
- No new preview and no new confirm. A watcher-driven tidy is the tidy that
  already exists.

## What is not in this slice

- **The daemon.** Slice 10.
- **Watching a share properly.** Decision 5: polled, not watched.
- **Desktop notifications.** DESIGN §10 has an opinion about them and it is not
  urgent; History and the window's own list are enough to judge this by.
- **Per-folder sweep intervals.** One global figure until somebody wants two.
- **Watching the duplicate pass.** Duplicates stay something you ask for.

## Verification

By hand, on a governed folder in `enforce`: drop a file in and watch it filed
after the cooldown; drop a file that keeps growing and watch it held back until
it stops; move a file by hand and confirm the resulting events do not cause a
second survey; delete the folder's rules and confirm the trouble is reported
rather than crashing the loop. On an `observe` folder, confirm nothing moves
and the count appears. Turn the switch off and confirm it stops.

Automated: the event-to-root mapping, the expected-set suppression, the pending
deadline arithmetic and the sweep are all testable without a filesystem. One
integration test uses a real `notify` watcher against a temp directory with
generous timeouts, as DESIGN §12 asks for.

`cargo fmt`, `clippy -D warnings`, `cargo test`, `npm run build`, and CI on
Linux, macOS and Windows.

## Risks

- **Watch limits.** Linux has `max_user_watches`, and a large tree exhausts it.
  The failure has to be reported as trouble on that folder rather than as a
  dead loop, and the hourly sweep has to keep working when it happens.
- **FSEvents reports directories, not files.** On macOS an event may name the
  directory rather than what changed in it. The mapping is to the governed root
  anyway, so this costs precision the design does not use.
- **A tidy while somebody is working in the folder.** The cooldown is the only
  thing standing between filing a file and filing a file an application is
  still writing. It is 30 seconds by default, and this slice does not change
  that, but it is the first time the number matters without a person watching.
- **The first feature that acts on its own.** Everything shipped so far moved
  files only when told to. The blast limits, the journal and `undo` all apply
  unchanged, and property 1 keeps it to folders where somebody chose `enforce`.
