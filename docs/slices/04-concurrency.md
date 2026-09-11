# Slice 4: Concurrency, sensed rather than configured

**Goal:** move several files at once, at a rate the far side can actually
take, without anyone having to know what that rate is.

**Runnable outcome:** a drain to the NAS over FTP that is several times faster
than today, where the number of parallel transfers was decided by asking the
server rather than by guessing — and where getting it wrong costs a rejected
handshake, not a failed file.

**This brief is the spec.** The tour is in [04-tour.md](04-tour.md).

---

## Why sensing beats configuring

The first design here was a per-scheme default plus back-off: start at 2 for
FTP, halve on a `421`. That is safe, and it is still in this slice as a safety
net, but as the primary mechanism it has a flaw worth naming:

> maybe the better ux would be to detect the transfer limits first with some
> samples or inbuilt detection mechanisms then start/resume the transfers that
> way some files do not have to be the scape-goat and fail

Exactly right. Learning by failure means a real transfer has to die to teach us
the limit. Nothing is lost — the source is untouched and the journal records
it — but the run reports "3 files failed" when nothing was wrong, and a
part-uploaded gigabyte is thrown away to discover something a handshake could
have answered in a second.

So: **find the limit before moving anything.**

The concrete hazard this exists for is not hypothetical. OpenDAL's FTP pool is
sized **64**, and its own source documents the failure:

```rust
// `{ status: NotAvailable, body: "421 There are too many connections from your internet address." }`
```

We build without the retry layer, so a `421` surfaces as a straight failure. A
naive `--parallel 16` against a consumer NAS would not be slow, it would fail
files.

## Decisions

**Concurrency is a property of the connection, not a global setting.** A
mounted volume and an FTP server have nothing in common here. The number is
derived per connection, per run.

**Ramp on the real queue, gated by a handshake.** The run starts at a safe
floor and climbs. Before it will use a higher level, it opens one extra
connection and checks the far side accepts it — a `stat` of a name that will
not exist, which forces a real connection and returns `NotFound`. For this
purpose `NotFound` is a success: the question is whether the *connection* was
allowed, not whether the file was there. Only once the slot is proven does a
file go to it.

This is why no file is the scapegoat, and it is why nothing extra is written.
The alternative considered — transferring sample files at increasing
concurrency, timing them and deleting them — measures the data path more
directly, but it writes into somebody's destination to calibrate, leaves
litter if it dies mid-probe, and spends real minutes before any of the user's
files move.

**Ramping stops at the ceiling, not at a measured plateau.** Climbing while
throughput improves sounds better and is what adaptive-concurrency literature
does, but throughput across files of wildly different sizes over a NAS is
noisy, and a bad measurement makes a run slower while looking principled. The
ceilings here are low enough (4–8) that overshooting is not the failure mode
worth engineering against. Revisit with numbers from a real drain.

**Halve the probe result for FTP.** Each active transfer holds a control
connection *and* a data connection. A server that accepts eight logins may
still choke on eight simultaneous transfers, so the probed number is an upper
bound on sockets, not on files.

**Do not probe a local filesystem.** There is no connection limit to find; the
constraint is disk and bus. `fs` gets a flat default of 4, which helps a
network mount (round-trip bound) without thrashing a spinning disk.

**Keep the back-off.** A probe measures handshakes at rest; real transfers are
heavier, and a server's limits can change mid-run. A `421` or a refused
connection during the run still halves the limit for the rest of it. Probing
makes that the exception rather than the mechanism.

**Nothing is remembered between runs.** Agreed separately: no schema change, no
stale wrong guess to explain later. A probe is cheap enough to repeat.

**An explicit override stays, and does not disable the safety net.** Someone
with a real server should be able to say so. The automatic back-off still
applies, because the point is to protect the far side, not to win an argument.

## Shape

- **`tungstate-backend`.** `Capabilities` gains nothing; concurrency is not a
  capability of the storage, it is a property of the link to it. A new
  `probe_concurrency(&dyn Backend, ceiling) -> usize` lives beside the factory,
  because it needs to issue concurrent operations and the factory is what knows
  the scheme.
- **`tungstate-transfer`.** A `Governor` owns the current limit: a floor, a
  ceiling, a handshake gate before each promotion, and a halving on any
  connection-level error, after which it stops climbing for the rest of the
  run. `Progress` and `ConflictResolver` gain `Send`,
  which is what lets a worker report from another thread. Both are held behind
  a `Mutex` inside the run, so a conflict prompt is still asked once at a time
  and progress lines do not interleave. `carry` becomes a scoped thread pool
  over a shared queue.
- **Races to close.** `disambiguate` probes for a free name and two workers
  could choose the same one; it needs to claim atomically or be serialised.
  The orphan sweep's comment says a link runs one file at a time — recovery
  still runs before any worker starts, so it stays true, but the comment must
  say *why* rather than implying it by accident.
- **`tungstate-cli` / `tungstate-gui`.** `connection test` reports the detected
  limit. A run says how many it is using. `--parallel N` overrides.

## Tests

- The probe returns 1 for a backend that fails every concurrent operation, and
  the ceiling for one that accepts everything, using fake backends — no
  network.
- The probe never exceeds its ceiling however permissive the backend.
- A parallel run over `LocalBackend` transfers the same files, with the same
  hashes, as a sequential one, and the summary totals match.
- Two conflicting files at once produce two prompts in sequence, not two at
  once, asserted with a resolver that panics on re-entry.
- `disambiguate` under N workers never lands two files on one name.
- A cancelled parallel run stops cleanly, leaves no partials, and every
  unfinished file is still at the source.
- A `421`-shaped error mid-run reduces the limit and the run still completes.
- Ordering is still honoured: with one worker the order is exactly as before.
- FTP, Linux CI only: a real parallel drain, and a probe against vsftpd
  configured with a low `max_per_ip`.

## Acceptance criteria

- [ ] `cargo build --locked`, `cargo test --locked` green, parallel and
      single-threaded
- [ ] `cargo clippy --all-targets --locked -- -D warnings` silent
- [ ] `cargo fmt --all --check` silent
- [ ] CI green on Linux, macOS and Windows
- [ ] By hand: a drain over FTP measurably faster than sequential, with the
      chosen concurrency reported and no file failing to calibration
- [ ] By hand: a server capped low is detected, not crashed

## Out of scope

Bandwidth caps and reachability pause/resume, which were bundled into slice 4
on the roadmap and now follow it. Per-file multithreading — splitting one file
across connections — decided against separately: it breaks the single-pass
hash, per-file resume and the commit ordering, for a gain that parallel files
mostly captures. Remembering a limit between runs.
