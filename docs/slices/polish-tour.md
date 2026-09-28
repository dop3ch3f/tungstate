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

## Phase C: Connections, with S3 and SMB

Three commits of engine and one of window, plus a CI job each for S3 and SMB.

### S3 is one more OpenDAL service

`s3_operator` sits beside `ftp_operator` in `tungstate-backend-opendal`. The
bucket, region and endpoint live in the connection's `options` map, so the
journal needed no new columns for them. The access key id is the username and
the secret key goes in the keychain, as a password does.

Two details are worth knowing. OpenDAL is told `disable_config_load()` and
`disable_ec2_metadata()`: without them it would also read `~/.aws`, the
environment and a cloud metadata service, and a connection that quietly worked
because of some other program's credentials would stop working on the next
machine. And S3 has no rename, so it takes the no-rename commit path FTP
already uses.

The HTTPS client needed care. OpenDAL 0.59 moved its HTTP client into a
separate crate that has to be installed once per process. Its default brings
`aws-lc-sys`, a C library every release runner would have to build. `ring`
does the same job and is already in the build through Tauri, so
`install_https()` installs `ring` as the TLS provider and a reqwest client
built on it:

```rust
static ONCE: std::sync::Once = std::sync::Once::new();
ONCE.call_once(|| {
    let _ = rustls::crypto::ring::default_provider().install_default();
    ...
});
```

`Once` runs the closure the first time and never again, however many threads
arrive together. That fits something that must be set up exactly once per
process.

These dependencies are `optional = true` and switched on by the crate's `s3`
feature. A feature in Cargo is a named switch that can turn on optional
dependencies, and `#[cfg(feature = "s3")]` removes code when it is off. A
build without S3 then fails with `SchemeNotCompiled`, which names the problem,
instead of failing somewhere obscure.

### SMB is a crate of its own, and synchronous

`tungstate-backend-smb` implements the `Backend` trait over smb-rs. The plan
was to wrap its async client in a private runtime, as the OpenDAL adapter
does. smb-rs has a `multi_threaded` feature that builds a synchronous client
instead, so each `Backend` method just makes a few SMB requests on the thread
that called it. No runtime, no `dispatch`, no futures moved in and out.

`Settings` implements `Debug` by hand:

```rust
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("host", &self.host)
            ...
            .finish_non_exhaustive()
    }
}
```

`#[derive(Debug)]` would have printed every field, the password included, into
any log line that printed the settings. Writing it by hand leaves the password
out, and `finish_non_exhaustive()` prints `..` so the reader knows something
was left out.

Three things testing found:

- **Names SMB would read as paths.** On macOS and Linux a single file name
  can contain `\`, which SMB treats as a separator, and `:`, which starts an
  alternate data stream. `a\..\b` passes every check a local path would, then
  climbs out on the server. `path::parts` refuses both characters. Windows CI
  caught a test that assumed Unix: there `x\y` is already two ordinary parts.
- **A hang at 8 MiB.** Samba offers 8 MiB per request. smb-rs answers one
  such read and then never answers the next, past its own timeout. Every size
  up to 4 MiB worked and ran no faster than 1 MiB, so requests are capped
  there (`MOST_PER_REQUEST`), with a comment saying why.
- **A folder not made yet.** A sync lists its new member's folder before
  anything is in it. OpenDAL's FTP and S3 services answer "empty"; SMB said
  "not found" and the sync stopped. The link end's own folder now lists as
  empty, after asking the connection's folder again, so a share that has gone
  still stops the run.

### Settings are checked once, in the journal crate

`ConnectionSettings::problems()` returns what would stop a connection working
as data (`SettingsProblem::NeedsShare`, `NeedsBucket` and so on), each with
its sentence from `thiserror`'s `#[error(...)]`. The CLI prints them before the
password prompt; the window's form asks `settings_problems` and shows the same
sentences. `in_the_clear()` answers, for a real connection rather than a
scheme, what crosses the network unencrypted. An S3 connection is encrypted
unless its endpoint is plain `http://`; SMB is encrypted when the server asks
or the connection insists.

### A check is remembered, and a connection can be retired

Journal v14 adds `checked_at`, `check_ok` and `check_note`, written by
`record_check` from either surface, and `retired_at`. `remove_connection`
works like `remove_link` does for saved pairs: it tries the `DELETE`, and when
the database's foreign keys refuse because an old operation names the
connection, it retires the row instead and renames it `name#id`, which frees
the name. Before any of that, `connection_uses` names what would break (saved
pairs, syncs, transfers that stopped part-way) and those refuse the removal
outright.

`open_connection` opens a `Connection` value that may not be saved yet. That
is what lets the window's form check its settings before saving. `test_settings`
builds a `Connection` with id 0, and when a password was typed, a
`MemoryStore` holding just that password, so nothing touches the keychain
until Save.

### The window

- `screens/connections/`: the section, with its list, Check all, and the kinds
  form.
- `ui/PlacePicker.vue`: the one Choose a place sheet, with this Mac's usual
  folders, every connection and a folder browser. `browse = false` returns the
  place as soon as it is chosen, for a Transfer pane, which browses on its
  own.
- `Sheet.vue` keeps a module-level stack (a plain `<script>` block beside
  `<script setup>` runs once for the module, not once per sheet), and only the
  top sheet answers the keyboard.

### Checked by hand against the NAS

Direct SMB to `area51.local`, share `PlexMediaServer`, inside one dated test
folder: a move drain with read-back verification, `connection test`, a sync
that set a changed version aside, then removal of the folder, and a listing of
the share's top level identical to the one taken before. The NAS accepts
`encryption=required`.
