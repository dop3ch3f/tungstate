# Slice 4g: A stop that means stop, and a limit that can come back down

**Goal:** a run should be interruptible immediately rather than eventually, it
should say what it is actually doing rather than what it usually does, it
should notice when going wider is not making it faster, and it should never
delete an original on the strength of a hash without being told it may.

**Runnable outcome:** copy a folder of large files to a mounted NAS. The rows
say *checking* while they are checking and *copying* while bytes move, and the
"4 at a time" readout falls to 2 on its own if two were all the mount could
usefully take. Press **Stop now** mid-file: it stops within a chunk, and the
usual banner offers Resume or Clean up. Run it again over the drained folder:
it reports the files as identical rather than "already there", and — because
this was a move — asks before removing each original from this machine.

**This brief is the spec.** The tour is in [04g-tour.md](04g-tour.md).

---

## Four reports, from one session

> in addition to a stop after these files button we need a hard stop button
> […] that can then ask if a clean up is necessary or if a resume would like
> to be possible

> i see for copying transfers but they have been 0 bytes sent for long and
> even though we have 4 files copying the other 3 are just waiting for one to
> finish so its not really parallel […] in these type of cases no need to ramp
> up maybe a ramp down would be ideal

> PS it says already there after a file has been sent to a destination that
> has that file but i chose for it to replace it

The second and third are the same event, which is worth saying plainly because
it took reading `classify` to see it.

## What "0 bytes sent for long" actually was

`transfer_one` calls `Progress::starting` and the row turns to **copying**.
Only then does `classify` run, and when the destination already holds a file of
the same size, `classify` reads *both copies in full* — `digest(source)`, then
`digest(destination)` across the network — before deciding anything.

`digest` reports nothing. So the row claims to be copying, at zero bytes, for
as long as it takes to read two whole files, one of them over SMB. Four rows
doing that look exactly like four stalled transfers. Then the hashes match, the
file is recorded as already present, and no byte is ever sent.

The rows were not lying about being parallel. They were lying about what they
were doing.

## Decisions

**A run reports the phase it is in, not the phase it usually is in.**
`Progress` gains `checking(path, done, total)`, and `digest` takes a reporter
so the check has a moving bar like a copy does. `total` counts both reads, so
the bar spans the whole check rather than resetting halfway. A row that is
reading is *checking*; a row that is writing is *copying*. This is the whole
fix for the third symptom and most of the second.

**Stopping has two strengths, and they are different promises.**
`AtomicBool` cannot express two, so `cancellable` takes an `Arc<Stop>` holding
an `AtomicU8`:

| | promise | checked |
|---|---|---|
| `Halt::AfterThisFile` | nothing in flight is abandoned | between files |
| `Halt::Now` | stops within one chunk, and may leave a partial | inside the copy and check loops |

A hard stop abandons the file being written. That is the point of it, and it is
survivable for exactly the reason slice 4d built: the op stays `Intended`, the
partial is findable, and the existing stranded banner already offers Resume and
Clean up. So the kill switch needs no new recovery machinery — it needs to fail
in the shape recovery already understands.

**The limit can come back down, which means workers park rather than leave.**
Today a worker whose index falls outside the limit returns, and a returned
thread cannot be recalled — so the limit is a one-way valve. That is fine for a
handshake that only ever ramps up before the run. It is not fine for a
measurement that may be wrong.

So `thread::scope` spawns `ceiling` workers and a worker above the limit waits
on a `Condvar` instead of returning. The limit moves both ways, and the run
ends when the queue empties rather than when the workers happen to have left.

**Concurrency is judged by throughput, not by protocol.** The governor already
refuses to guess — it asks the destination for a slot rather than trusting a
number somebody typed. Ramping down is the same principle pointed the other
way: measure bytes per second at the current width, retire a worker, measure
again, and keep the arrangement that moved more.

- A window is `SAMPLE` seconds of wall clock at a settled width.
- Descend while a narrower run is within `KEEP_RATIO` of the wider one, because
  equal throughput on fewer connections is strictly better for the far side.
- Climb back one step and stop adjusting the moment a step is clearly worse.
- Never below one, never above what the handshake already agreed to.

Deliberately not doing protocol detection. `smbfs` in a mount table says
nothing about whether *this* server parallelises, needs per-OS code, and would
have to be taught every future backend. Measurement is one mechanism that
covers all of them, which is the same argument the handshake already won.

**Changes take effect at file boundaries.** A worker three gigabytes into a
copy cannot stand down, and interrupting it to satisfy a measurement would
throw away the work. Over a batch this converges within a few files; over a
batch of exactly four enormous files it does nothing, and neither would
anything else.

**An identical copy is reported as identical, and in a move it is asked about.**
`classify` short-circuits to `Identical` before the resolver is consulted, so
`Replace` is never asked. Keeping that is right — re-sending bytes that are
already correct costs the whole transfer and changes nothing on disk — but two
things about it were wrong:

- "already there" does not say that both copies were read and compared. It now
  says so.
- `record_already_present` then applies the source policy, which on a move
  **deletes the original with no prompt**. A hash match is good evidence, but
  it is the tool silently destroying the user's only other copy on the strength
  of its own arithmetic, and it is the scariest thing this program does.

So `ConflictResolver` gains `identical(&Identical) -> IdenticalAction`,
consulted only when the policy would remove the original. It is **defaulted to
`DeleteOriginal`**, which is today's behaviour, so unattended runs and every
existing implementation keep working — a scheduled drain that stopped
reclaiming space because nobody was there to answer would be a worse failure
than the one being fixed. The window overrides it and asks, with apply-to-all.

`FileOutcome::AlreadyPresent` gains an `Original` telling the caller which
happened, because "identical, original removed" and "identical, original kept"
are different outcomes and the row has to be able to say which.

## Not in this slice

Bandwidth caps and reachability pause/resume, still owed from slice 4.
