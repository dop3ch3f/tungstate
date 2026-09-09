# Slice 3: The durable drain

**Goal:** move files from a full laptop to a NAS, one at a time, verifying each before deleting the original, and surviving the lid closing at any moment.

**Runnable outcome:**

```
tungstate link add ~/Videos /Volumes/nas/inbox --name laptop-to-nas --move
tungstate link run laptop-to-nas
```

Interrupt it, run it again, and it picks up without re-copying what already landed and without losing anything.

**This brief is the spec.** The walkthrough of the shipped code is in [03-tour.md](03-tour.md).

---

## Why this is the slice the project exists for

A MacBook full of videos has to reach a NAS. Finder's cut and paste is all-or-nothing and drops metadata. `rsync --remove-source-files` gets closest but has no per-file record, so an interrupted run cannot tell you what actually made it. FTP clients queue but neither verify nor remove.

The requirement is narrow and nobody serves it: copy one file, prove it arrived, delete the original to reclaim the space, move to the next, and be safely interruptible between any two of those steps.

## Decisions

**Links live in the journal database.** A link is source, destination, and options, and it has to persist so a drain resumes after a restart. Schema migration v2 adds a `links` table. This widens the journal crate from "provenance" to "persistent state", which is a deliberate scope change worth naming.

**Resume is per file.** Everything already committed is kept; only the file that was in flight is redone from the start. Byte-offset resume waits for slice 4b, where FTP makes partial transfers actually expensive. Over a mounted volume, redoing one large file costs minutes, and the code that would track and validate partial writes is where subtle corruption bugs live.

**Conflicts ask, when there is someone to ask.** Identical content at the destination is not a conflict; it means the file already transferred, so the source is removed and the operation is journaled as skipped. Differing content with the same name raises a prompt offering rename, skip, replace or quarantine, plus an "apply to all" so a hundred conflicts do not mean a hundred questions.

**Unattended, a conflict quarantines and the drain carries on.** When stdin is not a terminal, or `--on-conflict` was passed, there is no prompting. The incoming file lands under `.tungstate-quarantine/` at the destination, the source is removed, space is reclaimed, and the count is reported at the end. Nothing is lost and the drain does not stall while nobody is home.

**Verification defaults to `hash`.** The stream is hashed with BLAKE3 while it is being written, then the destination's size is checked. `size` is the weak option, `readback` re-reads the destination and compares digests and is the only level that proves the bytes on the NAS disk are the bytes sent. `readback` costs a second full read over the network.

**Source handling is explicit.** `--move` deletes the original after verification, `--move --trash` sends it to the OS trash, `--copy` leaves it. There is no default; omitting all three is an error, because guessing wrong either fails to reclaim space or deletes something the user wanted kept.

**Destination mirrors the source tree.** `Videos/2024/a.mp4` becomes `inbox/2024/a.mp4`. There is no policy engine until slice 5, so mirroring is the honest behaviour rather than inventing a layout now.

**Largest first by default.** Reclaiming space is the goal, so the biggest wins land soonest. `smallest-first`, `oldest-first` and `discovered` are available.

**Sequential.** One file at a time. Concurrency, bandwidth limits and reachability pausing are slice 4.

**Recently modified files are skipped.** A file written within the cooldown, thirty seconds by default, is left for the next run. Copying something still being written produces a corrupt destination and a verification failure at best.

## Shape

New crate `crates/tungstate-transfer`, plus a `links` module and migration v2 in `tungstate-journal`.

- `Link`: id, name, roots, `SourcePolicy`, `VerifyLevel`, `Order`, unattended `ConflictAction`, cooldown.
- `ConflictResolver` trait with an interactive implementation that caches an "apply to all" choice, and a fixed implementation for unattended runs and tests.
- `Transfer::run()` drives the per-file sequence: journal the intent, copy to a temp name, verify, rename into place, journal the outcome, then apply the source policy.
- Recursive walking lives here rather than on `Backend`, because ordering by size requires collecting the list anyway. Bounded by file count, not content. Slice 6's streaming walk is for million-file folders.
- Temp files are named from the operation id, so a crashed run's leftovers are identifiable and removable from the journal alone.
- Resume: on start, delete temp files belonging to interrupted operations, mark those operations failed, and re-queue their files. A re-queued file whose destination already matches is then a no-op.

CLI: `link add`, `link list`, `link run`, `link status`.

## Tests

- Happy path: files land, hashes match, sources are gone.
- `--copy` leaves sources; `--move --trash` does not hard-delete.
- **Interruption:** kill a transfer mid-file, restart, and assert the destination is complete and correct with no duplicates and no lost bytes. The test the whole slice exists for.
- A source deleted only after verification passes: inject a verification failure and assert the source survives.
- Identical file already at the destination is treated as transferred, not re-copied.
- Conflicting file triggers the resolver; each action is tested with a scripted resolver.
- "Apply to all" stops further prompting.
- Unattended quarantine puts the file under `.tungstate-quarantine/` and still removes the source.
- Cooldown skips a file modified a moment ago.
- Ordering strategies produce the order they claim.
- Empty source directories are pruned only when tungstate emptied them.
- Every test uses temp directories and a temp journal, so the suite stays parallel-safe.

## Acceptance criteria

- [ ] `cargo build --locked` clean, `cargo test --locked` green
- [ ] `cargo clippy --all-targets --locked -- -D warnings` silent, real `# Errors` docs
- [ ] `cargo fmt --all --check` silent
- [ ] CI green on Linux, macOS and Windows
- [ ] A real drain between two temp directories, verified by hand as well as by tests
- [ ] An interrupt-and-resume test that fails if the source is deleted before verification
- [ ] No `unwrap()` in `src/`

## Out of scope

No concurrency, bandwidth limits, or reachability pausing; those are slice 4. No byte-offset resume. No FTP; that is slice 4b. No policy engine deciding layout; slice 5. No dedup across the destination beyond the exact-match check; slice 8.
