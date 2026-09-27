# Polish tour: loose ends, the transfer queue, and the window screen by screen

The daemon is on hold and the terminal UI is dropped (see the 2026-09-27 entry
in [SYLLABUS.md](../SYLLABUS.md)). This pass polishes what the window already
does. It runs in four phases, and this file grows a section per phase and a
paragraph per screen. The before and after pictures are kept out of the repo.

The previous tour is [09f-tour.md](09f-tour.md).

---

## Phase A: loose ends

Three commits:

- The syllabus marks 10 and 10b on hold and 11 dropped, and says why.
- The release notes header no longer says organising folders isn't built.
- The folder watcher ignores file reads.

The watcher fix is the only code. On Linux, notify reports a file being opened
or closed as an `EventKind::Access` event. A survey reads every file in the
folder, so each survey woke the folder it had just looked at. Slice 9e found
this in the sync loop and filtered it there with `changes()`. That function now
lives in `tungstate-watch`, which `tungstate-sync` already depends on, so the
two loops share one answer:

```rust
pub fn changes(kind: EventKind) -> bool {
    !matches!(kind, EventKind::Access(_))
}
```

`matches!` is a macro that asks "does this value fit this pattern?" and gives
back a `bool`. `EventKind::Access(_)` is the pattern: the `Access` variant with
anything inside it.

The regression test needed a seam. The loop used to handle a batch of events
inline, inside `watch()`, which blocks on a real watcher. That handling is now
a function of its own, `take_in()`, which the test calls with a hand-made read
event and then a create event. Without the filter the read stirs the folder and
the test fails; with it, only the create does.

## Phase B: transfers as a download queue

### What was wrong

The engine already queued a second transfer behind the first. `RunQueue` held
individual links, and the worker took them one at a time. The window didn't
know that. It had one ledger, and three things went wrong:

1. Pressing Move ran `clearRun()` before asking the engine anything, which
   blanked the running transfer's list while it was still copying.
2. When the worker reached the second transfer, its `planned` event replaced
   the ledger with its own files.
3. The worker added up every leg of every transfer and sent one `done` at the
   very end, so the summary counted both runs.

### A job, and one event that says whose files come next

`RunQueue<Link>` became `RunQueue<Arc<Job>>`:

```rust
struct Job {
    id: u64,
    links: Vec<Link>,
    measured: Mutex<Option<(usize, u64)>>,
}
```

A job is what a person thinks of as "a transfer": one direction, or both
directions of an exchange. It carries a number so the window can point at it.

`Arc` is a reference-counted pointer that can cross threads. Three places hold
the same job: the queue, the worker once it takes it, and the thread that counts
its files while it waits. `Arc::clone` hands out another pointer to the one job
and copies no data. The job is freed when the last holder drops it.

`measured` sits behind a `Mutex` because the counting thread writes it while
the window's listing reads it. The rest of the job never changes after it is
made, so only that field needs a lock.

The worker now finishes each job before it looks at the next:

```rust
while let Some(job) = state.queue.next() {
    if cancel.asked() { break; }
    *lock(&state.running_job) = Some(Arc::clone(&job));
    let _ = worker.emit("transfer://job", JobView::of(&job, &state.journal));
    announce_queue(&worker);
    let (total, failure) = run_job(&job, &state, &mut resolver, &mut progress, &cancel);
    ...emit done or error for this job...
    if total.cancelled || cancel.asked() { break; }
}
```

`transfer://job` is the idea borrowed from sync's `sync://leg`. Transfers run
one at a time, so the file events already arrive in order. The window only
needed to know when one transfer ends and the next begins. After that event,
every file event belongs to that job until its `done` or `error`. The window
starts a fresh ledger on each `job` event and moves the old one to Recently
finished on `done`. Neither side needs a job id on every progress event.

The `break` rules are the two decisions you made. A failure or a vanished
destination belongs to that job only, and the loop moves on. Stop ends the
loop, and `queue.clear()` afterwards empties everything still waiting. The
`cancel.asked()` check at the top catches a Stop pressed in the gap between
two jobs, where no transfer is running to notice it.

### Two small additions to `RunQueue`

```rust
fn waiting_list(&self) -> Vec<T>
where
    T: Clone,
{
    lock(&self.waiting).iter().cloned().collect()
}

fn remove(&self, which: impl Fn(&T) -> bool) -> Option<T> {
    let mut waiting = lock(&self.waiting);
    let at = waiting.iter().position(which)?;
    waiting.remove(at)
}
```

`where T: Clone` sits on the method, not the whole `impl`. The sync queue's
`Job` isn't `Clone`, so a bound on the whole type would have stopped it
compiling. With the bound on the method, only a queue that calls
`waiting_list()` needs clonable items. For the transfer queue `T` is
`Arc<Job>`, so a clone is one counter increment.

`remove` takes a closure (`impl Fn(&T) -> bool`) instead of an id. That keeps
`RunQueue` generic, and the caller says how to recognise the item:
`|waiting| waiting.id == job`. The `?` after `position` returns `None` early
when nothing matches. That includes a job the worker has already taken, and
`remove_from_queue` answers `false` in that case.

### Reading two locks in the right order

The listing reads the waiting list and then the running job. The worker takes a
job off the list and only then records it as running. In the other order, a
listing that landed between those two steps would miss the job entirely. In
this order it can see the job twice at worst, so `queue_view` filters the
running one out of the waiting list. The comment in `queue_view` says why,
because swapping the two lines looks harmless.

### Counting a waiting transfer without touching its destination

`tungstate_transfer::measure(source, only)` expands the chosen files and adds
up their sizes. It asks only the source. The running transfer is using the
destination, and over FTP an extra connection opened just to size the queue
could be the one the server refuses, which the running job's governor would
read as a limit. The count runs on its own thread (`measure_later`) and
announces the queue again when it lands, so the row goes from "counting…" to
"212 files, 1.4 GB" without the window asking.

### `#[serde(flatten)]`

`done` now has to name its job, but the window already knew the shape of
`SummaryView`. Flattening puts the summary's fields beside `job` at the top
level of the JSON:

```rust
struct JobDone {
    job: u64,
    #[serde(flatten)]
    summary: SummaryView,
}
```

The window reads it as `{ job, ...summary }`, which is exactly how the
TypeScript type is written: `Summary & { job: number }`.

### The window

- `useTransfer.ts` keeps the running job's ledger and the queue as the engine
  last announced it, plus a list of finished jobs (`Ran`), newest first, capped
  at eight. History holds the rest.
- `planned` now adds to the ledger instead of replacing it. The two legs of an
  exchange plan one after the other inside one job, and replacing meant the
  first direction's files vanished halfway through.
- The ledger moved into `Ledger.vue` so the running transfer and each finished
  one draw it the same way.
- `start_transfer` returns `Accepted` instead of a count. The window knows
  straight away whether it started or joined the queue, and only switches to
  Runs if it started.
- `scripts/transfer.test.mjs` drives the store with the fake engine: two jobs
  keep their own files, joining the queue leaves the running ledger alone, an
  exchange's second leg adds rows, a failure stays in its own row, and only
  eight finished jobs are kept.

In the harness, `?scene=drain-queue-replay&step=1..5` replays the whole
two-transfer sequence as the engine emits it, and `?scene=drain-queue` shows a
full queue.

### Found in the real window

Driving the real window afterwards found what the harness could not. A
transfer of 10,000 small files froze the window for about half a minute while
the files themselves arrived fine. Every file event scanned the list for its
row, copied the whole list and redrew every row, three times per file. That is
quadratic, and it predated the queue.

- Rows are found through a `Map` from path to rows (`byPath`), which is O(1)
  instead of a scan.
- `touch()` republishes the list at most once per animation frame. Rows are
  changed in place, so nothing is lost between frames.
- The ledger draws at most 200 rows (`lib/inview.ts`). In arrival order it
  shows the ones where the run is working, and a line says how many there are
  in all. A filter or a sort starts from the top.

The regression test counts how often the list is republished during a burst of
1,000 events. It saw 1,000 before the fix and one after. Timing would have been
flaky; a count is not. It was checked again by hand at 100,000 files: the
window stayed responsive, Remove took a waiting transfer out, and Stop ended the
run and cleared the queue.

The same session found that a pair run twice reported "Copied 0 files, 0 B"
when every file was already there. The one-line result now says so.
