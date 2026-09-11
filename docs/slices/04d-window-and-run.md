# Slice 4d: The window and the run

**Goal:** stop the window and the drain being the same thing. A run that was
interrupted must be findable and finishable, and closing the window must not
silently kill work in progress.

**Runnable outcome:** kill the app mid-drain, reopen it, and the Transfers tab
says what was interrupted with a **Resume** and a **Clean up** beside it.

**This brief is the spec.** The tour is in [04d-tour.md](04d-tour.md).

---

## Why this slice exists

It was not planned. It came out of the first real use of the desktop app, which
found three things in one session.

The window could not show a running transfer at all, because Tauri resolved an
empty ACL and denied `event:listen`. That is already fixed. Chasing it exposed
the two this slice is for.

**Closing the app stops the drain, and nothing says so.** There is no daemon
until slice 10; the engine runs inside whichever process started it. Observed
directly: the process was gone, the operation was still `intended`, and a
2.9 GB `.part` sat frozen at the destination.

**An interrupted one-off transfer is unreachable and its partial is
permanent.** Three things combine. A browser transfer creates a link with
`saved = 0`. `Journal::links` returns only `saved = 1`, so it never appears in
"Run a saved pair…". And recovery is scoped per link —
`incomplete_for_link(self.link.id)`. So re-selecting the same files makes a new
link with a new id, whose recovery sweep cannot see the old operation. The old
row stays `intended` for ever and the partial is orphaned on the NAS. Repeat
that a few times and gigabytes accumulate where nothing will ever look.

The second is the serious one: it is silent, it costs real space on the
destination, and the user has no way to find it.

## Decisions

**Interrupted work is offered, never acted on.** The Transfers tab opens with
what was interrupted and two buttons. Resuming starts a multi-gigabyte transfer;
cleaning up deletes something. Neither is a thing to do to somebody while they
are reading. Automatic cleanup was considered and rejected: it would silently
throw away the 2.9 GB already copied.

**Finding interrupted work is a query, not a schema change.**
`Journal::incomplete` is already global and every op carries its `link_id`, so
this is a join. Nothing in the database changes, and no migration is needed.

**`saved` keeps meaning what it means.** It answers "did the user ask to keep
this pair?", which is the right question for the saved-pairs list and the wrong
one for "is there unfinished work?". A browser link stays unsaved and becomes
resumable through the interrupted list instead. Making browser links `saved = 1`
would have been one line and would have filled the list with
`browser-1789128635946-0`.

**Cleaning up is the recovery sweep without the copy.** `Transfer::recover`
already removes a partial, sweeps a backend's own orphan temporaries, and marks
the operation failed. Discarding is those three steps and then stopping. The
shared part moves into one function so the two cannot drift — a sweep that
matches in one path and not the other is how a file gets deleted that should
not have been.

**The window and the run are the same thing, and closing one ends the other.**
Hiding the window so a drain could continue invisibly was built and then
backed out. It buys a half-daemon: on Windows and Linux there is no dock icon,
so a hidden window has no way back until the run ends, and "the app is still
doing something, somewhere" is a worse thing to be unsure about than "closing
stops it".

So closing stops it, plainly. That is only acceptable *because* of the rest of
this slice: the operation stays `intended`, nothing is lost, and the next
launch offers to finish or clear it. Making an interruption cheap is what
earns the right to make closing simple. A real daemon is slice 10.

## Shape

- **`tungstate-journal`.** `link_by_id`, and `interrupted() -> Vec<Interrupted>`
  where `Interrupted { link: Link, ops: Vec<Op> }`. A join over
  `ops.status = 'intended'`, grouped by link. No migration.
- **`tungstate-transfer`.** The partial-and-sweep logic comes out of `recover`
  into one shared function, and a public `discard(link, destination, journal)`
  that runs it and reports what it removed.
- **`tungstate-gui`.** Commands `interrupted`, `resume_interrupted`,
  `discard_interrupted`, and a banner in the Transfers tab. No window-close
  handler: the default behaviour is the decided behaviour.
- **`tungstate-cli`.** The same two verbs, because the CLI must not be the
  weaker client: `tungstate link resume` listing interrupted work, and
  `tungstate link discard <name>`.

## Tests

- `interrupted()` finds an unfinished op on an unsaved link, which `links()`
  deliberately does not return.
- It groups by link and ignores links whose work all finished.
- `discard` removes the partial, sweeps a backend-left orphan, marks the
  operation failed, and copies nothing — asserted by the destination gaining no
  new bytes.
- `discard` on a no-rename backend removes the partial under the real name,
  the same branch `recover` takes.
- After `discard`, `interrupted()` is empty.
- Resuming an unsaved browser link by name works, and its recovery sweep sees
  its own old operation.
- Cross-platform, no Docker: every one of these runs against `LocalBackend` and
  the `atomic_rename: false` fake already in the suite.

## Acceptance criteria

- [ ] `cargo build --locked`, `cargo test --locked` green in parallel and
      single-threaded runs
- [ ] `cargo clippy --all-targets --locked -- -D warnings` silent
- [ ] `cargo fmt --all --check` silent
- [ ] CI green on Linux, macOS and Windows
- [ ] By hand: kill a drain, reopen, resume it, and confirm the file completes
      and the partial is gone
- [ ] By hand: kill a drain, reopen, clean up, and confirm the space is back
      and nothing was re-copied
- [ ] By hand: close the window mid-run, reopen, and confirm the banner offers
      the interrupted file

## Out of scope

A daemon, so a drain survives closing the window: slice 10, and the reason
closing stops it today. Byte-offset resume, so a resumed file continues rather
than restarting from the beginning. A confirmation prompt before closing
mid-run — worth considering once there is evidence anyone loses work to it,
and a few lines when that day comes. Connections in the desktop app, which
moves to slice 4e.
