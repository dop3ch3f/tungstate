# Slice 4f: The queue is a real thing

**Goal:** a transfer should be able to say what it is going to do, how far
through it is, and — after an interruption — what is left of *what you asked
for*, rather than what it can infer from a folder.

**Runnable outcome:** tick fifteen files, start a move, kill the app during the
third. Reopen: "your batch of 15 — 2 done, 13 to go", listed. Resume finishes
those thirteen and nothing else. While it runs, the file in flight shows a
byte count that moves.

**This brief is the spec.** The tour is in [04f-tour.md](04f-tour.md).

---

## One cause, three symptoms

From a real session, described by the user better than the code explained it:

> when testing i checked multiple files in that folder and then after the
> interruption it didn't show that the queue was interrupted but instead the
> current active file that got interrupted. then after clicking finish […] it
> popped another file into the transfers which was part of what i checked
> (maybe it was by chance)

Everything there follows from one gap: **a link never records what it was asked
to move.** The selection exists only as a `Vec<PathBuf>` passed to a worker
thread. When the process dies, it dies.

- The journal writes a row per file *as that file starts*, so an interrupted
  run leaves exactly one row: the file in flight. The other fourteen were never
  written down anywhere, so the banner could only name one.
- Resume has no list to consult, so `resume_interrupted` passes an empty
  selection and `Transfer::run` walks the whole source root.
- No queue can be displayed because no queue exists outside a running process.

"Maybe it was by chance" is half right. Ordering is largest-first, so if the
biggest files were picked, the folder's largest *are* the selection and the two
coincide — until the picks run out and the walk carries on into files nobody
chose. A characterisation test, `resuming_a_link_moves_everything_under_the_
source_not_the_original_selection`, pins the behaviour as it stands today; this
slice is what flips it.

## Decisions

**A selection is stored, and its absence means "everything".** Migration v5
adds `link_files`. No rows for a link means the whole source root, which is
what a saved folder-pair means and what every existing link already means. So
the migration is additive and no row needs backfilling.

**`Transfer::run` consults it, so every caller is correct by default.** The
alternative — keep passing a selection in and remember to pass the right one —
is what produced the bug. `run_selection` stays for the one caller that has a
selection before a link exists, and the browser stops using it: it writes the
rows, then runs. The in-memory `selections` plumbing through `spawn_run` goes
away entirely.

**The queue is emitted once, after the walk and the sort.** `Progress` gains
`planned(&[File])`, called at the top of `carry` when the engine knows the
whole list and its order. That is the only moment the full plan exists, and
emitting it is what lets the window show what is next rather than what has
already happened.

**Per-file progress is throttled in the engine, not the UI.** `Progress` gains
`advanced(path, done, total)` from the streaming loop, which already runs once
per 1 MiB. A 4 GB file would otherwise emit four thousand events. The engine
holds them back to roughly four a second, because the thing that knows how
often this is worth saying is the thing doing the work.

**Nothing about verification, ordering or commit changes.** This slice adds
knowledge of the plan and reporting of progress. The per-file state machine is
untouched.

## Shape

- **`tungstate-journal`.** Migration v5: `link_files (link_id, path)` with the
  pair as the primary key so the same path cannot be queued twice. `set_files`,
  `files_for`, and `Link::selection` loaded with the link.
- **`tungstate-transfer`.** `Progress::planned` and `Progress::advanced`, both
  defaulted so existing implementations keep compiling. `run` uses the stored
  selection when there is one. Throttling lives beside the stream loop.
- **`tungstate-gui`.** The Transfers table is built from the plan, not from
  arrivals: every file listed as *waiting*, the current one *copying* with a
  bar, the rest settling as they finish. `start_transfer` writes the selection
  before spawning. The interrupted banner counts against the stored batch.
- **`tungstate-cli`.** `link run` prints the plan before it starts, and the
  per-file line shows a byte count.

## Tests

- A link with a stored selection moves exactly those files, not its siblings.
- A link with no stored selection still walks the whole root, so saved
  folder-pairs are unchanged. This is the characterisation test written in 4d,
  renamed: its behaviour was never wrong, only its application to a link that
  *did* have a selection nobody had stored.
- Resuming a link with a stored selection resumes the batch: the committed ones
  are recognised as already present, the rest transfer, nothing else is
  touched.
- `set_files` is idempotent — the same path twice does not queue it twice.
- Deleting a link removes its rows, so the foreign key does not strand them.
- `planned` fires once, before the first `starting`, with the whole list in
  transfer order.
- `advanced` reaches the total exactly once per file, and is emitted far fewer
  times than there are chunks.
- A v4 → v5 migration on a raw v4 database keeps its links and ops and gives
  them an empty selection, which means the whole root.

## Acceptance criteria

- [ ] `cargo build --locked`, `cargo test --locked` green, parallel and
      single-threaded
- [ ] `cargo clippy --all-targets --locked -- -D warnings` silent
- [ ] `cargo fmt --all --check` silent
- [ ] CI green on Linux, macOS and Windows
- [ ] By hand: tick several files, kill mid-run, reopen, and confirm the banner
      counts the batch and Resume takes only the batch
- [ ] By hand: a large file shows a moving byte count

## Out of scope

Transferring several files at once, and the setting for how many: that is
slice 4, agreed separately and next after this. Bandwidth caps and reachability
pausing, also slice 4. Byte-offset resume, so a resumed file continues rather
than restarting. Connections in the desktop app, slice 4e.
