# Slice 4f tour: the queue is a real thing

Read this next to the diff. Like 4d, this slice came from use rather than from
the roadmap — and this time the bug report was a better diagnosis than the
investigation that preceded it.

Files: migration v5 and two methods in `tungstate-journal`, two defaulted trait
methods and a changed `run` in `tungstate-transfer`, a rebuilt Transfers table
in the window, and a CLI that draws a progress line.

---

## Part 0: One cause is better than three symptoms

The report:

> when testing i checked multiple files in that folder and then after the
> interruption it didn't show that the queue was interrupted but instead the
> current active file that got interrupted. then after clicking finish […] it
> popped another file into the transfers which was part of what i checked
> (maybe it was by chance)

I had been treating this as three things: a resume that took too much, a
missing queue display, and missing per-file progress. It is two things, and the
first two of those are one:

**A link never recorded what it was asked to move.**

From that, everything follows. The journal writes a row per file *as that file
starts* — `begin()` lives inside `copy_to`. So an interrupted run leaves
exactly one row, the file in flight, and a batch of fifteen reports as one. And
with no list to consult, resume passed an empty selection, which means
`Transfer::run`, which walks the whole source root.

"Maybe it was by chance" was half right, and the half that is wrong is the
interesting half. Ordering is largest-first. If the biggest files were picked,
the folder's largest *are* the selection, so a whole-folder walk and the batch
produce identical output — until the picks run out and the walk carries on into
files nobody chose.

**The lesson worth keeping:** when three symptoms appear together, look for the
one missing fact behind them before building three fixes. I had already written
two of those three fixes as separate work.

## Part 1: Absence as the default

```sql
CREATE TABLE link_files (
    link_id INTEGER NOT NULL REFERENCES links (id) ON DELETE CASCADE,
    path    TEXT    NOT NULL,
    PRIMARY KEY (link_id, path)
) WITHOUT ROWID;
```

Three decisions in five lines.

**No rows means the whole source root.** That is what a saved folder-pair
means, and what every link written before this already meant, so the migration
needs no backfill and no existing row changes meaning. This is the same
reasoning as `NULL` meaning "local" in v4 — an additive migration cannot
half-finish, and an `UPDATE` over every row can.

**The pair is the primary key**, so the same path cannot be queued twice. That
is not tidiness: a duplicate would make the engine transfer one file twice and
the plan report a count that does not match reality.

**`ON DELETE CASCADE`**, so removing a link takes its selection with it.
Without that the rows outlive the link and nothing can ever reach them — which
is exactly the shape of the bug this slice exists to fix.

`WITHOUT ROWID` because the table is its primary key and nothing else; SQLite
stores it as one B-tree instead of two.

## Part 2: Consulting, not being told

The old signature threaded a selection from the caller all the way to the
worker thread:

```rust
spawn_run(&app, names, selections)?;
…
let chosen = selections.get(index).cloned().unwrap_or_default();
let outcome = if chosen.is_empty() { transfer.run() } else { transfer.run_selection(&chosen) };
```

`resume_interrupted` had nothing to pass, so it passed `vec![Vec::new()]`, and
an empty selection quietly meant "everything". The new version:

```rust
pub fn run(&mut self) -> Result<Summary> {
    let chosen = self.journal.files_for(self.link.id)?;
    let files = if chosen.is_empty() {
        walk::files(self.source)?
    } else {
        gather(self.source, &chosen)?
    };
    self.carry(files)
}
```

The whole `selections` parameter is gone from `spawn_run`. This is the point
worth taking away: **"remember to pass the right thing" is not a design, it is
a hope.** Every caller was correct except one, and the one that was wrong was
wrong in the direction of moving files nobody asked for. Making `run` fetch
what it needs removes the chance to get it wrong.

One small thing fell out. `gather` used to fail if a path could not be stat'd.
On a resumed transfer that is the normal case — the files already moved are
gone from the source — so it now skips them:

```rust
let Ok(meta) = source.stat(path) else {
    tracing::debug!(path = %path.display(), "already gone from the source");
    continue;
};
```

## Part 3: Defaulted trait methods, and why they matter here

```rust
pub trait Progress {
    fn planned(&mut self, _files: &[Planned]) {}
    fn starting(&mut self, path: &Path, size: u64);
    fn advanced(&mut self, _path: &Path, _done: u64, _total: u64) {}
    fn finished(&mut self, path: &Path, outcome: FileOutcome);
}
```

`starting` and `finished` have no body: every implementation must provide one.
`planned` and `advanced` have empty bodies, so adding them broke nothing —
`SilentProgress` in the tests, `CliProgress`, and the window's `EventProgress`
all kept compiling, and each opted in where it wanted to.

That is the ordinary Rust way to widen a trait without a breaking change, and
it is worth contrasting with what happened in slice 4b, where `BackendError`
gained variants and every `match` had to be revisited. Enums break exhaustively
on purpose; traits extend quietly on purpose. Knowing which you want is part of
designing the thing.

`Planned` is a new public struct rather than reusing `walk::File`, because
`walk` is private. A public trait method taking a private type does not
compile, and the fix — making `walk::File` public — would have exported the
engine's internal bookkeeping as API.

## Part 4: Throttling belongs where the knowledge is

```rust
if reported.elapsed() >= PROGRESS_INTERVAL {
    reported = Instant::now();
    self.progress.advanced(source, written, total);
}
…
// Once at the end regardless of the throttle.
self.progress.advanced(source, written, written);
```

The streaming loop runs once per megabyte, so a 4 GB file would emit four
thousand events. The window would spend longer painting than the disk spends
copying.

It would have been easy to let the UI throttle instead. That is wrong twice
over: every client would have to reimplement it, and the client cannot know
what a sensible rate is — it does not know the file size, the chunk size, or
the transfer speed. **The thing doing the work knows how often the work is
worth mentioning.**

The unconditional final emit is the detail that matters. Without it a bar stops
wherever the last tick happened to land — 97%, say — and a bar stopped near the
end reads as a stall, which is precisely the impression this slice exists to
remove.

## Part 5: Bytes, not files

```ts
const progress = computed(() =>
  totalBytes.value ? (movedBytes.value / totalBytes.value) * 100 : 0);
```

The old bar counted files done over files total. For a real drain that is
badly misleading: fifteen 3 MB files and one 4 GB file means the bar reaches
94% in seconds and then sits there for ten minutes. Weighting by bytes, with
the in-flight file contributing its partial count, makes the bar mean what
people assume it means.

Each row also carries `done`, and only the row in flight draws a bar. A row
that is waiting or finished says so in a word — decoration on settled rows is
noise, and the whole point is to make the *active* one findable at a glance.

## Part 6: Two clients, one standard

The CLI got the same two things, because a command line that tells you less
than the window is a command line people stop trusting.

```rust
fn advanced(&mut self, _path: &Path, done: u64, total: u64) {
    if !self.interactive || total < 8 * 1024 * 1024 || done == total {
        return;
    }
    …
    self.redraw(&line);
}
```

Two guards worth noting. `interactive` is `IsTerminal` on stdout: rewriting a
line with `\r` is right for a person watching and turns a piped log into one
unreadable smear. And a file under 8 MiB never draws progress, because a bar
that appears and vanishes within one frame is worse than no bar.

`redraw` pads to the previous width rather than using an ANSI clear code — it
is shorter, it works on a Windows console, and the first version of this used
cursor-movement escapes that I could not convince myself were correct.

## What to look at in the diff

1. `tungstate-journal/src/schema.rs` — five lines of SQL and the comment
   explaining why absence is the default.
2. `tungstate-transfer/src/lib.rs` — `run`, and the `selections` parameter that
   is no longer anywhere.
3. `tungstate-transfer/src/tests.rs` — `a_stored_selection_is_the_only_thing_
   moved` and `resuming_a_stored_selection_finishes_the_batch_and_nothing_else`,
   which are the bug written down twice.
4. `tungstate-gui/ui/src/App.vue` — the `planned` listener that builds the whole
   table before the first byte moves.
