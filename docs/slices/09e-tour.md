# Slice 9e tour: kept in step while the app is open

Set a sync's **When it runs** to **Keep in step while the app is open**. Save
an export into `~/CapCut` and it is on the NAS a few seconds later, with
nothing pressed. Change something on the NAS and it reaches the laptop the
next time the NAS is looked at. Delete something, and the sync stops and waits
for you instead of carrying the deletion by itself.

Brief: [09b-folder-sync.md](09b-folder-sync.md), section 9e. The 9d tour is
[09d-tour.md](09d-tour.md).

---

## 1. A trigger, not a second engine

The brief's rule for this slice: nothing new decides what moves. The new code,
`tungstate_sync::follow`, answers one question, *when is this sync worth
running?*, and hands the run back to whoever called it:

```rust
pub fn follow(
    followed: Vec<Followed>,
    pace: &Pace,
    handle: &Handle,
    told: &mut dyn FnMut(&Heard),
    run: &mut dyn FnMut(&str) -> Outcome,
) -> Result<(), String>
```

`run` is a closure the caller passes in. The window's closure runs the sync
through the same gate and queue as pressing Run. The command line's
(`tungstate sync follow`, section 7) runs it in the terminal. The daemon in
slice 10 will be the third caller. `&mut dyn FnMut(&str) -> Outcome` means
"any function-like thing that takes a sync's name, may change its own state,
and says how it went".

## 2. When a sync is due

- **A folder on this Mac is watched.** Every event pushes the run out by the
  sync's cooldown, so a file still being written keeps putting the run off.
  That is the watcher's own `Schedule`, reused as it is: a second save inside
  the cooldown restarts the wait rather than causing a second run.
- **A folder elsewhere is listed.** No event arrives from another machine, so
  it is listed every 30 seconds while it changes, and each quiet look doubles
  the wait up to 10 minutes, as you chose. A listing is a digest of every
  path, size and time (`follow::listing`), so no file is read.
- **Every sync runs when the loop starts**, because files that changed while
  nothing was following them sent no event anybody heard.
- **And on the hour**, whatever was heard. Watchers drop events.

The pace is data, not constants:

```rust
pub struct Pace { pub quick: Duration, pub slowest: Duration, pub retries: Vec<Duration>, ... }
```

`Pace::default()` is your numbers. The tests pass a `Pace` in milliseconds, so
the whole loop, real folder watching included, runs in a few seconds.

## 3. A run's own writes are not news

A run copies a file into a watched folder, the watcher reports it, and without
care that report starts another run. Two things stop it:

- **Before a run, every path it will write is expected back.** `written()`
  reads them off the decided plan (copies, parked versions, set-asides,
  renames, and every folder above them), in every spelling the watcher might
  use (`/tmp` and `/private/tmp` on macOS). The watcher's `Echoes` set ignores
  them for ten seconds. The engine's half-written `.tungstate-N.part` files
  are ignored by name.
- **After a run, every listed member is listed again**, and that listing is
  the new starting point. What the run wrote on the NAS is then not a change
  the next look finds.

## 4. A run nobody pressed never removes anything

You chose *pause and ask*. `follow::unattended` is the rule, shared by the
window and the command line:

```rust
pub fn unattended(opened: &Opened, decided: &Decided) -> bool {
    let plan = &decided.plan;
    plan.reversible()
        && !plan.blast.values().any(|b| b.removing > 0)
        && crate::refusals(opened, decided).is_empty()
}
```

When it says no, the caller answers `Outcome::Held`. The loop stops running
that sync, even on the hour, until `Handle::resume` is called. The window
shows a notice on the sync's page, **waiting for you** in its row, and a line
on the rail. Once you have looked at the preview and run it, it is resumed.
Replacing an old version with a newer one sets the old one aside, like any
run, and is not held.

## 5. Out of reach is said once

A member that cannot be listed is **paused**, said once, and tried again at
1, 2, 5, then every 10 minutes (your numbers). It is never reported again
until it comes back. When it does, the sync runs, since anything may have
changed while it was away. The pace is a small type of its own, `Poll`, pure
enough to test to the minute without a network.

The window also says a *run's* trouble once. If deciding a run fails because
the NAS has gone, the loop tries again in 15 seconds, and the same error is
not sent again until it changes.

## 6. Keeping in step, in the window

- **The fourth choice** under **When it runs**. It is stored in the `continuous`
  column that has been in the journal since v12 and was never written until
  now. `Launch` gained `Continuous`, so there is still one setting, not a
  combination of bools.
- **Each folder's state on the sync's page**: *watching for changes*,
  *checked every 2 minutes*, or *out of reach, tried again on its own*.
  `following_state` and `sync://following` carry it, and a state is only sent
  when it changes.
- **The loop starts when the window opens**, and again whenever a sync is
  made, changed or removed, so it always follows what the journal says.
- **A run it starts goes through the same gate as Run and as a transfer.** It
  claims the sync queue itself (`RunQueue::claim`, new), because it runs on
  its own thread. When it finishes it picks up any Run you pressed meanwhile,
  which otherwise would have waited for ever (found by a test, section 9).
- **A background run does not pull the screen away** from what you are
  looking at. Only a run you started moves on to its result.

## 7. The same loop in the terminal

```
tungstate sync follow [NAME...]
```

It follows the named syncs, or every one set to `--on-launch continuous`
(also new), and holds the terminal, printing each member's state and each run.
It is SEAM rule 2, the terminal can do what the window can. It is also how
the loop was checked against the NAS without taking your screen over.

## 8. By hand, against the NAS over SMB

The laptop was a local folder and the NAS a folder on the mounted share,
which counts as elsewhere and is listed. The cooldown was three seconds.

- **The opening run** carried the file already on the laptop.
- **A save on the laptop** arrived on the NAS 7 seconds later, byte for byte,
  in **one** run: the copy landing did not start another.
- **Two saves 1.5 seconds apart**: one run, and the NAS holds the second.
- **A file made on the NAS** reached the laptop about 90 seconds later. The
  NAS had been quiet, so it was being listed every 2 minutes. The next listing
  saw it and listing went back to every 30 seconds.
- **A deletion on the laptop**: *its next run would remove files, so it
  waits*. The NAS copy was untouched.
- **The share unmounted**: *out of reach* after 24 seconds, then nothing at
  all for the next three and a half minutes of retries. Remounted, it was
  listed again at the next retry, about three minutes later, back to every
  30 seconds, with nothing pressed.

The test folder was removed, the share's top level matched the listing taken
before, and the share was unmounted again.

## 9. What checking found

- **By hand: a file still being written when the loop started was never
  carried.** The opening run correctly left it alone, but a file that has
  stopped changing sends no event, so nothing came back for it until the
  hour. A run that leaves anything "still being written" now asks for another
  once the cooldown has passed (`Outcome::Done { unsettled }`). There is a
  test for it.
- **By test: a Run pressed while a kept-in-step run held the queue was queued
  and never picked up.** The loop's run now drains the queue before letting go.
- **By reading, in the 9d code:** the guard that frees the queue if a run
  panics also freed it on a normal exit, after the next worker might already
  have claimed it. It is now disarmed on a normal exit.
- **A test that raced the clock.** The first "still being written" test
  counted runs, and under the load of the whole workspace testing at once its
  opening run sometimes started after the file had already settled: the file
  still arrived, in one run instead of two. It now asserts only the arrival,
  and a second test drives the loop with a run that reports "unsettled", so
  the rerun is checked with no clock to race. Three full workspace runs and
  repeated runs of the sync crate were clean after that.

## 10. What is checked

- **9 in `tungstate-sync`** for following:
  - the pace, to the minute;
  - an unreachable member said once, and its retries;
  - what a run writes being expected back;
  - a saved file arriving with nothing pressed, its own copy starting nothing;
  - two saves making one run;
  - a member that cannot be listed pausing once and running when back;
  - a held sync waiting through a sweep and running once resumed;
  - a file still being written at the start being carried once it settles,
    and a run that left something unsettled being run again with no event.

  The timing ones were run repeatedly, in parallel and one at a time.
- **1 in the window crate** (claiming the queue), **1 in the journal** (the
  fourth launch value), **1 in the CLI** (`sync follow` with nothing to follow,
  and `--on-launch continuous`).
- **Screens:** `sync-following` (watching, and out of reach), `sync-held`,
  and the settings sheet with the fourth choice, in the headless harness.

## What is still not verified

- **The real window**, for this slice and 9d. The loop was driven by hand
  through `sync follow`, which is the same code, but no one has left the app
  itself open through an afternoon of exports. That needs you.
- **FTP as a followed member.** Listing goes through the same backend as
  everything else, but no FTP member has been followed.
- **A folder watched by both the folder watcher (slice 9) and a sync.** A
  governed folder that is also a sync member can be tidied while a sync writes
  into it; the tidy does not take the gate.
