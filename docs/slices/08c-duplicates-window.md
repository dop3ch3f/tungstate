# Slice 8c: the Duplicates window

**Goal:** point Tungstate at a folder or a drive, watch it find what is there
twice, choose what to keep, and clear the rest. Without a terminal, and
without ever being told a saving that is not real.

**Runnable outcome:** open Tungstate, click Duplicates, point it at a folder,
watch it work, tick what to clear, press the button, and undo it.

Engine and command line shipped in [8b](08b-dedup.md) and its
[tour](08b-tour.md). This slice is the window, and the seam additions it needs.

---

## What the engine already gives us

- Groups of identical files, each with every copy, the copy to keep, and the
  reason it won.
- Folders copied whole, as one decision.
- Two names for one file, reported as what they are.
- A sampled mode for network volumes, with `sure: false` and a `confirm` step
  that reads in full only what is about to be acted on.
- Digests remembered in the journal, and hashes a transfer already recorded
  reused for free.
- A plan, which the executor applies and `undo` reverses, and a refusal for
  the one action that cannot be reversed.

## What this slice adds

### 1. Seam commands, and the first progress a folder pass has ever had

Five commands, all `async` (slice 8's lesson: a synchronous command runs on
the main thread on macOS and freezes the window):

| Command | Answers |
|---|---|
| `find_duplicates(root)` | starts a scan; returns when it has an answer |
| `duplicates_progress` (event) | how far through, and what it is reading |
| `cancel_duplicates` | stop a scan that is taking too long |
| `clear_duplicates(root, choices, extras)` | confirm, plan, apply, journal |
| `duplicate_settings` / `set_duplicate_settings` | the remembered answer to "set aside or trash" |

**Why an event and not a spinner.** A drive scan is not a folder tidy: it can
take minutes, and the person needs to know it is still working and roughly
where it has got to. `docs/SEAM.md` lists *progress for a long tidy* as a
known gap; this fills half of it, for the pass that needs it most. The shape
copies the transfer events, which the window already knows how to draw.

The scan runs on a worker thread and reports: files seen, files sampled, files
read in full, bytes read, and the path it is on. Cancellation is the drain's
`Stop` flag, which already exists.

### 2. Scanning a folder nobody governs

`find_duplicates` takes a bare path, the way `learn_folder` and
`compare_folder` do, and surveys with the probe policy. Duplicate-finding has
nothing to do with rules, so requiring a governed folder would be a rule
invented by the plumbing. The command line's `dedupe` keeps needing one for
now; the window does not.

### 3. The screen

Its own section in the sidebar, with its own colour and its own pixel mark,
because this is a thing people open on its own rather than a tab inside
Organize.

**Before a scan.** One button, `Point at a folder…`, and the folders scanned
before it, so a second look at Downloads is one click.

**While scanning.** What it is doing, in the window's own words: *Looked at
4,210 files. Sampled 180. Read 12 in full.* A Stop button, because a scan of a
drive is the one thing here somebody will want to abandon.

**After.** Groups, largest saving first. Each group shows every copy with its
path, size and date, the one to keep marked, and why it won. Anything matched
on samples says *almost certainly the same, confirmed before anything moves*.
Folders copied whole come first, as one row each. Names that are two names for
one file are listed separately, with *dealing with one frees nothing*.

**Choosing.** Per group: click any copy to keep that one instead. Across all
groups: *keep the newest*, *keep the oldest*, *keep the one in this folder…*.
Nothing is ticked by default beyond the engine's own choice, and the total at
the bottom says what clearing would free.

**Acting.** One button, and it says which action it will take, because that
choice was made once and is remembered: `Set 214 files aside` or `Move 214
files to the Trash`. The confirm sheet names the number, the bytes and the
action, and for the trash says plainly that only Finder can bring them back.
Afterwards, the result and, when it can be, `Put it back`.

### 4. What the window must never do

1. **Never offer a saving that is not real.** A hard-linked name is not a
   copy, and its size is not reclaimed.
2. **Never act on a sample.** A group matched on samples is confirmed in full
   before a single file moves, and a copy that turns out to differ is dropped
   and said out loud.
3. **Never lose the last copy.** A group always keeps one, whichever copy the
   person picks, and the kept copy can never be one of the ones cleared.
4. **Say which action, before it happens.** Set aside and trash are different
   promises: one is undoable here, the other is not.
5. **Files are not operations.** The count on screen is files, never ops.

## Decisions I would like you to confirm

1. **A scan of a whole drive.** Pointing at `/` would walk everything,
   including system directories, and take a long time. I propose refusing
   above the home directory, and warning past roughly 200,000 files with a
   Stop always available. The alternative is to allow it and let people wait.
2. **What "keep the newest" means for a folder copied whole.** A folder has no
   single date. I propose the newest file inside it, and to say so in the
   words on the button.
3. **Whether the window may scan a connection** (the NAS over FTP) in this
   slice. The engine supports it; the window's folder picker does not, so it
   means a "Go to…" control like the transfer panes have. I would leave it to
   a later slice and keep this one about local folders and mounted volumes,
   which is where the space actually is.

## What is not in this slice

- **Near-duplicates.** Slice 8d.
- **Thumbnails.** A duplicate finder for photos wants them; the window has no
  image pipeline, and reading a thumbnail out of every file is the
  expensive thing this whole design avoids. Worth its own decision later.
- **Scanning several folders at once.** One root at a time, as the engine has.
- **Automatic clearing.** Nothing here runs without being asked.

## Verification

By hand, on the demo folders and on a real folder of the user's choosing: a
scan that finds nothing, a scan with files and folders duplicated, a hard link
present, a cancelled scan, a scan of a large folder, choosing a different copy,
each pick-by-rule, set aside and its undo, and trash and its refusal to undo.
At 860×560 as well, and in all three themes.

`cargo fmt`, `clippy -D warnings`, `cargo test`, `npm run build` (the CSS
checker and `vue-tsc`), and CI on Linux, macOS and Windows.

## Risks

- **A long scan is the first thing in the window that can be cancelled
  mid-flight.** The drain's stop flag is proven; using it here means the scan
  has to check it between files and leave nothing half-done. It only reads, so
  the worst case is a wasted minute.
- **Memory.** A group holds every copy's path. A drive with a million files
  and heavy duplication could hold a lot of strings. Measured before it ships.
- **The window will show a number people act on.** Every count on that screen
  is a promise about somebody's files, which is why properties 1 to 3 are
  written down before any code.
