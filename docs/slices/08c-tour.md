# Slice 8c tour: the Duplicates window

Point Tungstate at a folder, a drive or a connection, watch it look, choose
what to keep, and clear the rest. This is the walk through what shipped and
what driving it by hand found.

Brief: [08c-duplicates-window.md](08c-duplicates-window.md). Engine:
[8b](08b-dedup.md) and its [tour](08b-tour.md).

---

## 1. The first folder pass that says anything while it runs

Every other folder command in this project returns in silence. A tidy of 5,000
files takes seconds; a scan of a drive takes minutes, and a window that says
nothing for minutes looks hung. `docs/SEAM.md` has had *progress for a long
pass* listed as a known gap since slice 7e. This fills it for the pass that
needs it most.

The mechanism is the one the transfers already use: the command runs on a
worker thread and emits an event the window listens for.

```rust
let reporter = app.clone();
dupes::scan(&target, &state.journal, &state.scan, move |progress| {
    // Dropped rather than raised: a window that cannot hear progress is
    // not a reason to abandon a scan that is working.
    let _ = reporter.emit("dupes://progress", progress);
})
```

**A closure passed as an argument** is how the engine side stays ignorant of
Tauri: `scan` takes `impl FnMut(&ScanProgress) + Send` and does not know
whether that draws a screen, counts for a test, or does nothing.

## 2. A stop that is its own

```rust
pub struct Scan {
    stop: Arc<AtomicBool>,
}
```

`Arc` is a pointer several threads may hold at once; `AtomicBool` is a flag
they may read and write without a lock. Together they are the smallest thing
that lets a button on one thread end a loop on another.

It is deliberately **not** the drain's `Stop`. That one is shared by a running
transfer, and stopping a scan must not stop a drain that is halfway through a
40 GB file.

Two details worth the words:

- **Starting a scan clears the flag.** Otherwise a stop from last time kills
  the next scan before it reads a byte. The test found this the hard way: it
  set the flag before calling and the scan ran to completion, which is correct
  behaviour and a wrong test. It now presses Stop from inside the progress
  callback, which is the only way a person can press it.
- **A stop is not a failure.** The digest returns a sentinel string, `dupes::
  STOPPED`, which `find` turns into `Trouble::Stopped`. Reusing the trait's
  error type would have made "you pressed a button" indistinguishable from "a
  file could not be read", and the window says very different things about
  those two.

## 3. Watching and stopping live in the reader, not in the thinking

`tungstate_core::dupes` still does no I/O and takes no callbacks. Both the
watcher and the stop hang off `Cached`, the thing that actually reads:

```rust
let mut digest = Cached::new(backend, journal, &root)
    .watched_by(Box::new(move |watch| { … }))
    .stopping_when(Box::new(move || stop.load(Ordering::Relaxed)));
```

**`Box<dyn FnMut(&Watch) + Send + 'a>`** is "some closure, chosen at runtime,
that may be called many times, safe to send to another thread, living at least
as long as the digest". Clippy asked for names for those, so there are two type
aliases, `Watcher` and `Asked`, which also read better at the call site.

Both are checked once per file, in `note()`, which is also where the count of
files looked at comes from.

## 4. Scanning anything, including the NAS

```rust
let end = tungstate_journal::ends::parse_end(target, None, journal)?;
let backend = tungstate_backend_opendal::open(&end, journal, &secrets())?;
```

`parse_end` is what the transfer side already uses, so `~/Movies` and
`nas:incoming` are both valid targets and neither needs new parsing. The
snapshot comes from the probe policy, as `learn` does: **finding duplicates has
nothing to do with rules**, so requiring a governed folder would have been a
rule invented by the plumbing.

Over a connection the pass samples rather than reading every byte (8b, §10b),
and the desktop's trash is not offered, because it is this machine's. That
refusal lives in the command rather than in the window: a command that can be
called any other way must not try to trash a path it cannot reach.

## 5. Clearing runs the pass again

The window does not send back the groups it was shown. `clear_duplicates`
takes the ids of the ticked groups and the paths of any copy picked by hand,
then scans again, confirms, plans and applies.

That sounds wasteful and is not: the digests are remembered (8b, §6), so the
second pass over an untouched folder reads nothing. What it buys is that the
folder is acted on **as it is now** rather than as it was when the list was
drawn, which is the same reason `apply` refuses a stale plan.

Two small rules inside it:

- Pins are filtered to the groups being acted on. A pin for a group left alone
  would change which copy a later pass keeps, and nobody asked for that.
- Every ticked group is confirmed byte for byte first. Any copy the samples
  got wrong is dropped, counted, and reported on the result screen.

## 6. The kept copy is a copy

`Group.keep` was a path and nothing else, so the window drew every copy's date
except the one they were all being compared against. The engine now sends
`kept: Copy` as well, which is the same three fields as every other copy. It is
also what lets *keep the newest* consider the copy that is currently kept:
without it, the rule could only choose between the extras.

"Keep the newest" is **files only**. A folder has no single date and inventing
one would be a rule nobody asked for.

## 7. The window

`state/useDupes.ts` holds one scan: a phase, what was found, what is ticked
(`Set<string>` of group ids, which is why the engine gives every group a stable
id), and which copy is kept where that is not the engine's choice.

Everything ticks on arrival, because somebody opening a duplicate finder came
to clear space; untick what you want to keep. Two counts are computed from the
ticks, and both are **files, never operations**: a plan also removes the
directories it empties, and slice 7b shipped "moved 9 file(s)" for five moved
files.

The action button says what it will do, because the choice is made once and
remembered in the settings table: `Set aside 4 file(s)` or `Send 4 file(s)`.
The first time, it asks, and explains that one is undoable here and the other
is the desktop's.

## 8. Driving it by hand found three things the harness could not

The headless harness photographs any screen, which is how the layout was
checked. It cannot tell you that a screen has no way out of it.

1. **A result was a dead end.** With a scan finished, the only ways off that
   screen were to clear something or scan again. "Look somewhere else" now sits
   beside the heading.
2. **A sentence about the wrong thing.** The previous run's *Put 2 files back
   where they were* stayed on screen above a fresh scan.
3. **The list of places looked at before did not exist.** The brief asked for
   it; it was simply missed. It is in the settings table, newest first, without
   repeats, capped at six.

What it also confirmed, on real files: two folders scanned, a folder copied
whole reported as one row, a three-copy group, a copy picked by hand and
honoured, four files set aside with the picked copy left alone, the emptied
directories cleaned up, and every file returned by Put it back.

**A note on driving a Mac from a script**, since this is the third slice to hit
it: macOS kept returning focus to the browser, and the window swallows the
first click after being activated. Every input was preceded by a check of which
application was frontmost, and skipped if it was not ours.
`docs/slices/08-tour.md` §7 has the rest of the method.

## 9. What is checked

Five tests in `crates/tungstate-gui/src/dupes.rs`, against real temporary
folders rather than fixtures, because this is the layer where the backend, the
journal and the pass meet:

- a scan finds the copy, counts extra *files*, reports what it would free, and
  reports progress as it goes;
- clearing sets the extra copy aside and leaves one where it was;
- the copy you pick is the one that stays;
- a group nobody ticked is left alone;
- a scan stopped mid-flight says `stopped` and changes nothing.

Plus `npm run build` (the CSS checker and `vue-tsc`), `cargo fmt`, `clippy -D
warnings`, 499 tests, and CI on Linux, macOS and Windows.

## What is still not verified

- **A connection has never been scanned.** The code path is there and the
  sampling it turns on is tested in the core pass, but nothing has pointed the
  window at a real NAS over FTP. It needs a connection this machine can reach.
- **Only at 1080 by 720, and only in the retro theme.** The smallest window
  size and the other two themes have not been looked at for these screens.
- **Memory on a very large scan.** A group holds every copy's path; a drive
  with a million files and heavy duplication has not been tried, and the brief
  said it would be measured before it shipped. It has not been.
- **The progress event is untested.** The scan test proves the callback fires;
  nothing proves the window draws what it emits, because that needs the real
  window and a folder big enough to watch.
