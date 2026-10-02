# Slice 8e tour: finding duplicates faster over a network

Gemini 2 was slow finding duplicates on the NAS, and the question was
whether tungstate's duplicate finder would do better. Read on
2026-10-02, the code said it would not, and could do worse. Over a network
the time goes on round trips and on bytes moved, not on hashing, so being
written in Rust saves almost nothing by itself.

This slice measured first, then fixed four things, one commit each, and
measured again after each.

## The numbers

One generated folder of 5,000 files, measured after each fix. "Requests" and
"read" are what crossed to the storage on a first scan; "warm" is a second
scan of the same, unchanged folder. Rows are cumulative: each includes the
fixes above it. The fake network is the table to trust for a real NAS (why
below). On this machine's disk the pass reads every candidate in full, not
samples, so its "read" column is mostly that.

**Fake network (20 ms a request, 50 MB/s)**

| | requests | read | time | warm requests | warm time |
|---|---:|---:|---:|---:|---:|
| Before | 24,774 | 109.7 GB | 43.9 min | 12,696 | 5.0 min |
| 1. Ranged reads | 24,774 | 855 MB | 9.9 min | 12,696 | 5.0 min |
| 2. List without reading | 19,774 | 528 MB | 7.9 min | 7,696 | 3.0 min |
| 3. Size and time from the listing | 13,735 | 528 MB | 5.5 min | 5,683 | 2.3 min |
| 4. Several at once | 13,737 | 528 MB | 3.0 min | 5,683 | 2.3 min |

**SMB directly (local Samba in Docker)**

| | requests | read | time | warm requests | warm time |
|---|---:|---:|---:|---:|---:|
| Before | | | did not finish in 90 min | | |
| 1. Ranged reads | 24,774 | 855 MB | 12.0 min | 12,696 | 4.7 min |
| 2. List without reading | 19,774 | 528 MB | 7.6 min | 7,696 | 13 s |
| 3. Size and time from the listing | 13,735 | 528 MB | 7.4 min | 5,683 | 10 s |
| 4. Several at once | 13,737 | 528 MB | 4.6 min | 5,683 | 8 s |

**SMB mounted (same server)**

| | requests | read | time | warm requests | warm time |
|---|---:|---:|---:|---:|---:|
| Before | | | not run | | |
| 1. Ranged reads | 24,774 | 855 MB | 11.9 min | | not finished |
| 2. List without reading | 19,774 | 528 MB | 7.3 min | 7,696 | 8 s |
| 3. Size and time from the listing | 13,735 | 528 MB | 7.4 min | 5,683 | 8 s |
| 4. Several at once | 13,737 | 528 MB | 2.2 min | 5,683 | 8 s |

"Did not finish" and "not finished" are runs stopped by the two-hour limit
on background jobs. The mounted "before" state was not run, for the same
reason the direct one did not finish.

**This machine's disk**

| | requests | read | time | warm requests | warm time |
|---|---:|---:|---:|---:|---:|
| Before | 32,542 | 162.4 GB | 59 s | 14,638 | 2 s |
| 1. Ranged reads | 32,542 | 53.6 GB | 43 s | 14,638 | 2 s |
| 2. List without reading | 27,542 | 53.2 GB | 42 s | 9,638 | 0 s |
| 3. Size and time from the listing | 15,677 | 53.2 GB | 41 s | 5,683 | 0 s |
| 4. Several at once | 15,677 | 53.2 GB | 8 s | 5,683 | 0 s |

## How it was measured

Nothing touched the NAS: a local stand-in was chosen instead. `dupes_bench`
(`crates/tungstate-cli/examples/dupes_bench.rs`) makes a folder shaped like
a photo library and runs the duplicate pass over it exactly as the window
does: list, then sample or read, then group. It runs each pass twice. The
second run is "warm": the digests the first run worked out are in the
journal.

The folder is 5,000 files: 4,000 originals and 1,000 extra copies of some of
them (20%). Photos are 1.5 to 9 MB and videos 20 to 300 MB, 144 GB in all.
Each file is zeros except three 4 KiB blocks of noise at the start, the
middle and the end, so the videos are sparse and cost almost no disk. APFS
fills the holes in files under about 32 MB, so the photos did cost what they
weigh: 21 GB, deleted afterwards. One original in fifty has the same size as
another file and is not that file. Half of those differ everywhere; the other
half differ only in the middle, where no sample looks, so sampling calls them
the same and only a full read tells them apart.

It ran four ways:

- **A local folder.** Read in full, because a local disk is not networked.
- **A fake network.** The local backend wrapped in `Slow`, which waits 20 ms
  per request and pushes every byte through one shared 50 MB/s pipe. Time
  then follows requests and bytes the way a NAS's does, and repeats exactly.
- **SMB, directly.** tungstate's own SMB backend against the Samba container
  CI uses, run locally in Docker.
- **SMB, mounted.** The same share mounted with `mount_smbfs`, read through
  the local backend, which notices the mount is networked.

The fake network is the one to read. Requests and bytes decide the speed
over a real network; the seconds in the two SMB tables are a Docker container
on the same laptop, which has no real round trip to pay.

`tungstate dedupe --timings` prints the same table for any folder you run it
on, so you can take these numbers to the NAS yourself.

## Counting: a wrapper that is the backend it wraps

`tungstate_backend::counted` holds two things. `Tally` is a set of counters
per `Stage` (listing, samples, whole files). `Counted` is a `Backend` that
holds another backend, forwards every call, and adds one to the requests of
whichever stage is running, plus the bytes that came back.

```rust
fn read_range(&self, path: &Path, offset: u64, len: u64) -> Result<Vec<u8>> {
    self.bytes(self.inner.read_range(path, offset, len))
}
```

Every trait method is forwarded by hand, and it has to be: a
method left out falls back to the trait's default, so a wrapper that forgot
`read_range` would quietly reintroduce the slow read it is there to measure.

The counters are `AtomicU64` with `Ordering::Relaxed`. Several threads add to
them at once (fix 4), and each counter is its own number, with nothing else
that has to be seen in step with it, which is exactly what `Relaxed`
promises and no more. Switching stage is different: it reads when the last
stage began and writes when the next one does, as one step, so that is a
`Mutex<Instant>`.

`open_read` hands back a reader that outlives the call. `CountedRead`
therefore holds an `Arc<Tally>` rather than borrowing one: a borrow would tie
the reader's life to the backend's, and `Box<dyn Read + Send>` is
`'static` by default, so the compiler refuses it.

Stages come from `Staged`, a `Digest` wrapper in `tungstate-execute` that
says "samples now" before passing a `partial` call through. The backend
cannot tell a sample read from a whole-file read, so the switch has to
happen one layer up.

## Fix 1: read samples as ranges

A network scan hashes four 64 KiB samples of each file whose size matches
another's: the start, a third of the way, two thirds, and the end. Only the
first was a ranged read. For the other three, `read_at` reopened the file
and read every byte up to the sample into `std::io::sink()`. Sampling a file
cost about twice its size. On the benchmark folder, 109 GB to sample 2,013
files.

`Backend` gains a method with a default:

```rust
fn read_range(&self, path: &Path, offset: u64, len: u64) -> Result<Vec<u8>> {
    // ...open_read, skip `offset` bytes, read `len`
}
```

A **default method** is a trait method with a body. Every type that
implements `Backend` gets it for free, so adding it broke nothing: test
doubles, the attrs crate's wrappers, anything outside this repository. The
default is correct and slow, the same as the old code. The three backends
that can do better override it:

- **Local** opens, `seek`s, and reads. Seeking past the end is allowed and
  reads nothing, which is the contract for a range after the file ends.
- **OpenDAL** asks for `range(offset..end)`. A range running past the end is
  refused outright by some services and comes back from FTP as a short read,
  so on any error it asks the size and tries again for what the file has.
  The size is only asked once a read has failed, so the ordinary case costs
  one request.
- **SMB** reads are already a request at an offset, so the reader just
  starts further in. One snag: `open_read` returns `Box<dyn Read + Send>`,
  and you cannot get the concrete `SmbRead` back out of that box to change
  its offset. So the opening moved into a private `reader()` that returns
  `SmbRead` itself; `open_read` boxes it, and `read_range` sets the offset
  first. `read_prefix` is now `read_range(path, 0, len)`; before, it streamed.

The regression test wraps a 10 MiB file in `Counted` and asks for a sampled
digest: at most 256 KiB may be read. Before the fix it read 21 MB.

## Fix 2: list without reading

The window listed a folder with the probe policy, which wants each file's
media type, so the listing read 64 KiB of every file, 5,000 reads and
328 MB, before duplicate finding began. Most of those files have a size no
other file has, so they are never looked at again.

`tungstate_attrs::survey_at(backend, policy, Tier::Stat)` lists and reads
nothing. `deepen(backend, &mut snapshot, &paths, tier)` reads what a tier
wants for the named files only. `tungstate_core::dupes::candidates` names
them: the files that share a size with another, the same set the pass will
sample. `survey` itself is unchanged for every other caller.

- The window lists at the stat tier. It already took a file's kind from its
  extension. "Looks alike" is the exception: it uses the media type, and it
  only runs on this machine, where reading it costs little, so with that
  ticked the listing still reads it.
- The command line lists first, then deepens only the candidates to whatever
  the folder's rules need. The rules decide which copy is "already where it
  belongs", and a file alone at its size is never in a group. A test builds a
  folder where only the media type says which copy is in place and checks
  that copy is kept; with `deepen` removed, the other one was.

`gather` was split for this into the `stat` and a `fill` that does the
reading, so `deepen` reuses `fill` and costs no extra `stat`.

## Fix 3: take size and time from the listing

The digest cache is keyed by path, size and modification time, so each
digest asked the storage for both, up to three `stat`s per file, for facts
the listing already had.

`Cached::knowing(&snapshot)` builds a `HashMap` from path to size and time,
and `stamp()` looks there before asking. The project's rule against
`HashMap` is about iteration order in the planner; this one is only ever
looked up in, so its order never shows.

Confirming a group before anything moves does **not** use it. A file edited
between the listing and the confirm would then be looked up under its old
size and time, find the old digest, and be called a duplicate. Confirmation
keeps asking the storage, so a changed file misses the cache and is read
again. The window's clear path used one `Cached` for both finding and
confirming, so it now makes a second one for the confirm.

A warm scan of an unchanged folder now makes no requests at all after the
listing.

## Fix 4: several at once

The pass asked for one sample or one file at a time. Over a network that
means waiting for each round trip before starting the next.

**The engine.** `Digest` gains `partials` and `wholes`, which take every
candidate at once and answer in the order asked. Their defaults call
`partial` and `whole` one at a time, so `similar`'s eye, the test tables and
anything else implementing `Digest` are unchanged. `find` now asks for all
samples in one call, groups the answers by size and digest in a `BTreeMap`
(so in the same order as before, whatever order they arrived in), then asks
for all whole files the same way.

**Who does what.** The journal is a SQLite connection, which is `Send` but
not `Sync`: it may move to another thread but two threads may not use it at
once. So the reading is split in two:

- `read()`, `sample()` and `every_byte()` are free functions that take a
  `&dyn Backend` and touch nothing else. They run on worker threads.
  `Backend: Send + Sync` is what lets several threads share one backend.
- Recall from the cache, remembering a result, progress and the stop check
  stay on the calling thread, which owns the journal.

**The workers.** `several()` uses `std::thread::scope`, which lets threads
borrow from the function's stack, the job list and the backend, because the
scope does not return until they have all finished. Each worker takes the
next job from an `AtomicUsize`, does it, and sends the answer down an
`mpsc` channel. The calling thread waits on the channel with
`recv_timeout(100 ms)`, so a press of Stop is noticed during a long read
rather than after it.

**How many.** It reuses the transfer engine's `Governor` (slice 4), so it
asks a NAS for no more than a copy would. Over a network it starts at two
and asks for one more connection at a time, up to four, with a `stat` of a
name that will not exist: a refusal costs a handshake, not a failed read.
On a local disk it is eight. A refusal during real work ("421", "too many",
"connection refused") halves the width and puts the file back on a retry
queue.

That retry queue is why a worker with nothing to do does not simply exit. A
refused file can come back after the others have finished, and the worker
that refused it may be the one the halving just parked. So an idle worker
naps for 10 ms and looks again, until the calling thread says the run is
over. Refused at a width of one, a file is an ordinary failure.

A property test generates folders with identical files, same-size files that
differ, and files that differ only in the middle, and checks that one at a
time, one at once and eight at once find exactly the same groups, sampled
and in full. Two tests cover the back-off, using a backend that answers its
first reads with an FTP `421`: at width four the pass recovers; at width one
it reports the file. Another checks that a stopped scan stops.

## Small things worth knowing

- `if let Err(error) = &answer && is_overload(error) && governor.limit() > 1`
  is a **let chain**, new in the 2024 edition: pattern matches and plain
  conditions joined with `&&` in one `if`.
- `let Some(at) = job else { ...; continue; };` is **let-else**: bind the
  value or leave the block. It keeps the happy path unindented.
- `Hashed` (a digest and the bytes it cost) exists so a worker can hand both
  back in one message, and the calling thread adds the bytes to its count.

## What the numbers say

- **On the fake network, a first scan went from 44 minutes to 3.** Almost
  all of the 44 was the sampling bug: 109 GB pulled through a 50 MB/s pipe.
  Ranged reads alone took it to 10 minutes; the other three fixes took it to
  3. A second scan of an unchanged folder went from 5 minutes to 2.
- **The same groups every time.** All five states found the same 919 groups
  on every network run. Sampling finds 15 more than a full read does: the
  files planted to differ only in the middle. They are marked unconfirmed,
  and anything about to be moved is read in full first, which drops them.
- **The listing is now most of the time.** On the fake network, 2.2 of the
  3 minutes are the listing: 683 directory listings and 5,000 `stat`s, one
  after another. That is the next thing to fix (below).
- **SMB in Docker has a slow read, most likely not tungstate's.** A 64 KiB
  ranged read took about 54 ms against a container on the same laptop, while
  a `stat` took 1.5 ms. macOS's own SMB client, through the mount, paid
  about the same, and a raw read inside the container took under a
  millisecond (possibly from its cache), so the cost is probably Docker
  Desktop's file sharing under Samba. Not proven. Reading four at
  once still cut the sampling from 7.3 minutes to 4.5 directly, and from
  7.3 to 2.1 through the mount. One idea was tested and ruled out: smb-rs
  sends each request as several socket writes with Nagle's algorithm on, which can stall for a
  delayed ACK. A scratch build with `TCP_NODELAY` set ran no faster
  (250 s against 268 s), so it was thrown away.
- **The "before" state over SMB did not finish.** It had 109 GB to pull
  through that slow read, and was stopped after 90 minutes.

## Also fixed: the benchmark's own clock

The first fake network slept after every 8 KiB read, and a sleep that short
overshoots by more than it lasts. It now owes time until there are at least
5 ms of it, and credits an overshoot to the next read. Every state was
rebuilt at its own commit and measured again with the corrected pipe; each
result was checked against the model (requests times 20 ms, plus bytes at
50 MB/s) and agrees to within the cost of a real sleep, about 23 ms a
request.

Runs left going overnight still crawled, several times slower than the
model, while the same runs by day matched it. Nothing in the code differs,
so the likely cause is macOS stretching short timers while nobody is at the
machine. The numbers here are from daytime runs under `caffeinate`.

## What is not done, and not verified

- **Nothing was measured on the NAS**, by choice. `--timings` is there for
  when it is: point `tungstate dedupe --timings` at a folder under the
  mounted share. The command line still only takes a local path, so direct
  SMB from the command line is not possible yet; the window does that.
- **The listing still asks for every file twice.** `survey` walks with
  `read_dir`, which already returns each entry's size and time, then calls
  `gather`, which `stat`s each file again. That is 5,000 of the 5,683
  listing requests. The SMB and local listings are complete enough to use as
  they are; OpenDAL's are not always (some services list without a size),
  so it needs a per-backend answer rather than a blanket change. It is the
  next request saving to make, and it touches every caller of `survey`, so
  it was left out of this slice.
- **FTP and S3 were not measured.** The ranged read and the governor apply
  to them; their CI suites cover correctness, not speed.
- **Hashing on the NAS itself**, which would move no bytes at all, needs a
  tungstate running there (slice 10b), which is on hold with the daemon.
- **"Looks alike" over a network** still refuses. It decodes every picture
  and video, so it would download everything.
- **Three folder-sync watcher tests fail on this Mac when the whole suite
  runs in parallel**, and did so on `main` before this slice too. They pass
  run one at a time, and passed in CI for alpha.7. Not touched here.
