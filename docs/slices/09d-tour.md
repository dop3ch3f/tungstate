# Slice 9d tour: syncs in the window

Open Tungstate, go to **Sync**, press **New sync…**, pick `~/Movies/CapCut`
and the NAS, choose **Both ways** and **Deletions too**. Preview a run, answer
any conflict it shows, press **Run**, and put the run back from the same
screen if it was not what you meant. Mark it to run when the app opens and it
does, showing you first, or quietly if you chose that and it would remove
nothing.

Brief: [09b-folder-sync.md](09b-folder-sync.md), section 9d. The 9c tour is
[09c-tour.md](09c-tour.md).

---

## 1. What you chose, and where it shows

- **Its own section**, called **Sync**, in teal, between Transfer and
  Duplicates. Teal is a new token in each of the three themes (`--tint-sync`,
  and `[data-half="sync"]` sets the main button's colour), bluer than
  Duplicates' green so the two never read as one.
- **The words.** The command line keeps `--push`, `--pull`, `--all` and
  `--exact`. The window says **Send from laptop**, **Bring into laptop**,
  **Both ways**, and **Deletions too** or **Only add**. Every sentence about a
  sync on screen comes from one file, `ui/src/lib/syncwords.ts`, so the same
  thing is never said two ways.
- **Conflicts are asked in the preview**, before anything moves, with **Keep
  laptop's**, **Keep nas's**, **Keep both** or **Leave it** per file, and the
  same for all of them in one click.
- **Everything the command line does**: make, change, add or take out a
  folder, preview, run, stop, forget a file, put a run back, remove.
- **On launch**, with a quiet option (section 6).

## 2. One set of checks for both surfaces

SEAM rule 2 says the window and the command line reach the same answer. The
checks for making a sync lived in the CLI's `sync.rs` as private functions,
so they moved to `tungstate-sync/src/setup.rs` and both call them.

A refusal crosses as data, not a sentence:

```rust
pub enum Refused {
    DeleteNeedsExact,
    Overlap(String, String),
    Taken(String),
    // ...
}
```

The CLI words it with its flags ("`--on-remove delete` needs `--exact`"). The
window gets it as `{ "kind": "overlap", "first": "laptop", "second": "nas" }`
and words it plainly. `#[serde(tag = "kind", rename_all = "snake_case")]` on
the window's mirror of the enum is what turns each variant into an object
with a `kind` field. Neither surface can drift from what is checked, because
neither checks anything.

`impl From<JournalError> for Refused` lets `?` turn a journal error into a
refusal on the way out. `?` calls `From::from` on the error, so one `impl`
saves a `map_err` at every call.

## 3. Nothing moves that was not seen

A preview decides a run; a run decides again, because time has passed. If the
two differ, the person would be running something they never saw. So the
preview carries a **fingerprint** of what it would do:

```rust
pub fn fingerprint(&self) -> String {
    let described = format!("{:?}", self.ops);
    blake3::hash(described.as_bytes()).to_hex()[..16].to_string()
}
```

and `run_sync` must be handed it back. The engine decides again, compares,
and refuses with `sync://error` of kind `changed` if anything differs. The
digest is stable because the plan is built from ordered maps only, and
`Debug` of these types contains no addresses.

Answering a conflict previews again, so the numbers on screen are always the
run's. An answered conflict drops out of the next preview, because the engine
has settled it, so the window keeps its own list of every conflict it has
shown (`seen` in `useSync.ts`). Your answer stays visible and can be changed.

## 4. Watching a run, and stopping it

The run reuses the transfer engine's progress events. `EventProgress` gained
a stream name, so the same code emits `transfer://started` for a drain and
`sync://started` for a sync, and a drain and a sync never fill each other's
lists.

The window also wants to know which pair of folders is copying. The engine
runs the preview's legs in order, and each leg is one transfer that reports
`began` once, so counting `began` is counting legs. `SyncProgress` wraps
`EventProgress` to do that and emits `sync://leg`.

Stopping needed the engine to listen. `run` gained a sibling, `run_until`,
that takes an `Option<Arc<Stop>>`:

- `Arc` is a shared pointer that can cross threads. The window's command
  holds one copy, and the thread doing the run holds another.
- `Option` because the command line has no one to press Stop.

A new function beside `run` rather than a new argument on it, so its callers
did not change. Anything a stop leaves undone is not remembered, and is
decided again next run.

## 5. Syncs and transfers take turns

Syncs run one after another, through the same `RunQueue` the Transfer section
uses. A sync never runs beside a transfer, because two runs writing one
folder at once would race on names.

The check has to be one decision, not two:

```rust
let _gate = lock(&state.gate);
if state.queue.is_running() {
    return Err("a transfer is running; ...".into());
}
```

`gate` is a `Mutex<()>`: a lock around nothing. What it guards is the
decision itself. A transfer and a sync each take it while they look at the
other and start, so both cannot find the other idle at the same moment. The
lock is released when `_gate` goes out of scope, which is Rust's way of making
"unlock" impossible to forget.

## 6. Running when the app opens

Journal v13 adds `syncs.run_quietly`. In the Rust it is not a second bool:

```rust
pub enum Launch { No, Ask, Quietly }
```

Two bools would allow "quietly, but not at launch", which means nothing.
Clippy refused a fourth bool on `Sync` and was right. The two columns stay
because `on_launch` came first and a journal is only ever added to.

When the window opens it checks for interrupted transfers, then previews each
marked sync:

- one set to run quietly runs at once, if it would remove nothing, needs no
  yes, and can be put back;
- everything else is gathered into one sheet. If any of them would remove
  files, the sheet leads with **Look at it first** and offers **Run anyway**
  second.

`tungstate sync set capcut --on-launch ask|quietly|no` does the same from the
command line.

## 7. History in one query

The section lists every run with copies and removals as separate columns.
`Journal::past_syncs` counts both, plus renames and bytes, in one `GROUP BY`
over the ops, rather than one query per run.

## 8. Counts that cannot be mixed up

The window already makes "a count of files" a branded type that only named
extractors can produce (`lib/counts.ts`). The sync added its own:
`arriving`, `removing`, `replacing`, `renaming`, `parking`, `syncCopied`,
`syncTookOff` and so on. "Copied" and "removed" are separate numbers on every
screen and are never added together, and the type checker refuses a raw
`.length` where one is wanted.

## 9. Looked at, not inferred

Every state was photographed in the headless harness (`mock.html`), with no
window and no focus taken from you. There are thirteen new scenes:

- `sync-start`, empty and with syncs;
- `sync-make`, `sync-one` (two folders) and `sync-four` (four);
- `sync-preview`, `sync-nothing`, `sync-conflict`, `sync-refused` (an empty
  member and a large removal) and `sync-deletes` (cannot be put back);
- `sync-running`, `sync-done` and `sync-put-back`;
- `sync-settings` and `sync-launch`.

The conflict preview was also checked in the paper and graphite themes.

A fresh-context critic that saw only the screenshots scored three rounds:
6, then 7, then 8 out of 10. What changed because of it:

- "taken off" became "removed" everywhere;
- anything that needs a yes now sits above the lists;
- removals are grouped, with the reason said once per group;
- the reading line only appears when reading is the slow part;
- the launch sheet leads with looking when a run removes files;
- radios take the section colour;
- one sync's own history lost its redundant column;
- a two-folder sync names the other folder ("deleted on laptop"), and a file
  a run could not do says why in plain words.

What it asked for and did not get, because you had already chosen otherwise
for the other sections: **Put it back** as the main button on a finished run,
the blue live progress bar, and the shared filter chips.

## 10. Where this departs from the brief

- **Conflicts are asked in the preview**, not through the transfer's conflict
  channel. A sync knows its conflicts before anything moves, and asking
  mid-copy would have meant pausing the engine. You chose this.
- **Syncs never run beside a transfer at all**, rather than only when they
  share a folder. The window cannot cheaply tell which folders a running
  transfer writes into.
- **`sync set --on-launch`** in the command line, so the window can do
  nothing the terminal cannot.
- **Launch without asking** is new (your choice), and it still asks when a
  run would remove anything.

## 11. What checking found

- The first launch sheet took Home's orange, because a dialog is teleported
  to the frame and outside the Sync section's colour scope. It now sets its
  own.
- The finished-run sentence ran together ("aside.9 files renamed"): Vue drops
  the space between optional blocks. It is built in the script now.
- The run-now sheet appeared over every scene until the fixture only marked
  syncs to run at launch in the scene that tests it.

## 12. What is checked

- **Rust:** 4 new in the window's `sync.rs` (a sync made from the form, a
  refusal as a `kind`, a preview listing what it removes, a conflict with each
  version); 3 in `tungstate-sync` (setup's refusals, a stop that remembers
  nothing it did not finish); 3 in the journal (`set_launch`, `past_syncs`
  counts, runs of a remade sync).
- **Window:** `npm run build` (the CSS checker, `vue-tsc` with the branded
  counts, the bundle) and `npm test`.
- 665 Rust tests in parallel and one at a time, `clippy -D warnings`, `fmt
  --check`, and CI on three platforms.

## What is still not verified

- **The real window, end to end.** Every screen was seen in the harness
  against a fake engine, and every command is covered by Rust tests against
  the real one, but no one has clicked through a real sync in the running app.
  That is the brief's "a day of use with no terminal open", and it needs you,
  or your go-ahead for me to drive the window.
- **The gate between syncs and transfers** has no test of its own; the queue
  it sits beside does.
- **Continuous sync** is 9e.
