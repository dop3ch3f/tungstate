# Slice 5b: the window catches up

**Goal:** everything the command line can do to a link or a connection should
be reachable from the window, because that is how this gets used.

**Runnable outcome:** launch the app, and never need a terminal for a day's
work. Make a link with a name, an order and a cooldown. See every link you
have, run one, preview one, change one, delete one. Check a connection and be
told what is in it and whether it will accept a file. Ask where a file went.
See what was set aside in quarantine.

**This brief is the spec.** The tour will be in `05b-tour.md`.

---

## The audit this came from

Three different kinds of gap, which is why "make the window match the CLI"
turned out to mean three different jobs.

**Commands the window has no equivalent for.** `explain` and `policy validate`,
which is the whole of slice 5.

**Commands the window has, that no screen calls.** `create_link` is wired all
the way through Rust, with a form carrying a name, an order and a cooldown, and
nothing in the interface reaches it. So is `whereis`. So is `quarantined`,
which slice 4e fixed and left unused.

**Options the window cannot express.** A transfer started from the browser
hard-codes `Order::LargestFirst` and `cooldown: ZERO`. There is no way to set
either, and no way to pass the `--parallel` ceiling.

And one gap made on 2026-09-14 while fixing the FTP write bug: `connection test`
on the command line now names what it found and reports whether the root
accepts files. The window still shows a count and a path, which is the exact
output slice 4e called "reassuring and useless".

## Decisions

### Split in two, and this is the first half

**Chosen.** This slice closes what bites during ordinary use. `explain` and
`policy validate` wait for **5c**.

**Rejected: one slice covering everything.** A policy does nothing yet. The
planner is slice 6 and the executor is slice 7, so a policy screen would be a
window onto a decision nothing acts on. Designing it after living with
`explain` on the command line will produce a better screen than designing it
now, and the link gaps are the ones costing time today.

### Policy screens, when they come, are read-only

**Chosen for 5c, recorded here so it is not relitigated.** The window shows a
policy, validates it with the same loader the command line uses, and draws the
diagnostic against the right line. Editing stays in a text editor.

**Rejected: an editable pane, and a form builder over the rules.** The policy
lives in the folder and is meant to be committed, so a person will also edit it
by hand; a window that owns the file fights that, and a form cannot represent
every policy the parser accepts. Read-only keeps one source of truth.

### A Links tab, not a dialog

Links are currently a by-product of a transfer: you pick files, you move them,
a `browser-…` link appears. That is right for a one-off and wrong for the
saved pairs you run repeatedly. They get a screen of their own, alongside
Connections, with the same shape: a table, an add-or-edit dialog, and the
destructive action guarded.

### Deleting a link is new to both surfaces

Neither the window nor the command line can delete a link, so this is not a
parity gap; it is a missing feature that the Links tab makes obvious. It ships
here with `tungstate link remove` alongside it, so the two surfaces stay level.

**A link with history is not deleted silently.** The journal's operations
reference a link by id, and losing the link would leave `log` and `whereis`
naming an id nobody can resolve. So a link that has ever run is *retired*
rather than removed: it stops being offered, and its history keeps working.
One that never ran is deleted outright. Both surfaces say which happened.

**Retirement is a `deleted_at` column, not the `saved` one** — the user's call,
and the right one. `saved` already answers a different question, whether this
is a named pair or the one-off a browser transfer makes, and a column that
answers two questions answers neither. `deleted_at` also records *when*, and
NULL means live, so every existing row is correct without backfilling.
Migration v6, additive.

**Retiring releases the name.** Keeping it reserved would mean an invisible row
refusing a name the user can see is free, which is a worse surprise than a
retired link reading as `nas#3` in the one place retired links are ever shown.

**A link with unfinished work is refused rather than removed.** Same rule as
`connection remove` while a link points at a connection: removing it would
strand a part-copied file at the destination with nothing left able to name it.
Run it or discard it first, and the error says so.

## What ships

1. **Journal** — migration v6 adds `deleted_at`, and `remove_link` decides
   between deleting and retiring. `links()` and `link_by_name` see live links
   only; `link_by_id` deliberately still finds retired ones, which is what
   keeps an operation's reference resolving.
2. **CLI** — `tungstate link remove <NAME>`, reporting whether the link was
   deleted or retired, and refusing nothing else.
3. **GUI, a Links tab** — every link with its source, destination, policy,
   verify level, order and cooldown; Preview, Run and Remove per row, and an
   **Add a link** dialog reaching the existing `create_link`. That last one is
   the command line's `link add`: a saved pair made without moving anything,
   which the window could not express at all. `preview_link` is new, because
   `preview_transfer` takes ticked files and settings from a dialog, while a
   saved link already carries both and remembers its own selection.
4. **GUI, the transfer dialog gains order and cooldown** — with the current
   hard-coded values as defaults, so nothing changes for anyone who ignores
   them. `TransferRequest` grows two fields.
5. **GUI, a parallel ceiling** — the `--parallel` override, on the run
   controls rather than in the link, because that is where it lives on the
   command line and it is a property of a run rather than of a pair.
6. **GUI, connection test parity** — `ProbeView` carries the entry names and
   the writability verdict, and the connections table shows them. The same
   rule, `Scheme::rootless_warning` and `probe_writable`, already shared.
7. **GUI, a Find tab** — `whereis` and `log` over one input: give it a path or
   a BLAKE3 hash and see everything that ever happened to it. The shape of what
   you typed decides which is asked, the same rule the command line uses, so
   one answer comes back however you ask. Both commands were already exposed;
   this is the screen they never got.
8. **GUI, quarantine** — on the same tab, per link: what the destination set
   aside rather than overwrote. `quarantined` has been correct since 4e and
   unreachable ever since.

## Acceptance criteria

- `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`,
  `cargo test --locked` clean, and `npm run build` clean.
- A saved link created in the window with a name, an order and a cooldown, and
  the same settings read back by `tungstate link list`.
- A link deleted from the window; one with history retired instead, with its
  `log` still resolving.
- `connection test` in the window showing the same names and the same
  writability verdict as the command line.
- A file found by hash in the Find tab.
- CI green on Linux, macOS and Windows.
- **By hand:** a day of ordinary use without a terminal.

## Tests

**Journal** — `delete_link` removes one that never ran; `retire_link` keeps the
row and stops `links()` offering it; an op's `link_id` still resolves after its
link is retired; deleting a link with history is refused in favour of retiring.

**CLI** — `link remove` on a fresh link and on one with history, reporting
which it did; an unknown name refused.

**GUI** — the pure functions, as the crate's convention has it: the link form
round-trips every field; the transfer request carries order and cooldown
through to `NewLink`; `ProbeView` reports names and writability. Screens are
proved by hand and by the command-line suite underneath them, which is the same
stated gap as slice 4e and for the same reason.

## Out of scope

- `explain` and `policy validate` in the window. Slice 5c.
- Editing a policy anywhere but a text editor. Decided above.
- `folder add`, which is still a stub on both surfaces and stays one until the
  planner gives a governed folder something to do.
- A queue that outlives the process. Slice 10.
