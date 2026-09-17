# Slice 7b: the window tidies

**Goal:** make the second half of tungstate usable by somebody who would never
open a terminal or hand-write a TOML file.

**Runnable outcome:** open the app, add a folder, pick a layout from a list,
look at what it would do to your files, click one button, and see it done —
with one more button that puts it all back.

**This brief is the spec.** The tour will be in `07b-tour.md`.

---

## Why this is 7b rather than the 5c that was promised

The 5b brief promised a **5c**: `explain` and `policy validate` on screen,
read-only. That was the right slice to schedule when it was scheduled, and its
own argument was *"a policy does nothing yet, so a policy screen would be a
window onto a decision nothing acts on."*

Slices 6 and 7 gave the decision something to act on. A screen that only shows
where one file *would* go is now the smaller and stranger half of what the
window can offer, so the slice grew into the loop: see the folder, see what
would change, change it, take it back. It is renumbered 7b because that is the
honest dependency — it needs the executor.

The read-only policy view survives inside it, exactly as promised, including the
diagnostic drawn against the right line.

---

## Decisions

### The window speaks plainly; the engine keeps its vocabulary

**Chosen, and it is the decision this slice turns on.** On screen: **Rules**,
**Preview**, **Tidy up**, **Put it back**, **Already tidy**. In the code, the
CLI, the journal and the docs: policy, plan, apply, undo, drift.

**Why.** A person opening this app has a messy Downloads folder, not a mental
model of a reconciliation controller. Four pieces of jargon before the first
useful action is four chances to close the window. The command line is the
reference surface and keeps the precise words, because that is where someone
goes when they need to be precise.

**The cost, stated so nobody is surprised by it:** two vocabularies for one set
of ideas, which makes the docs harder to map onto the screen. Paid for with a
glossary in the tour and one line of hover text on each button.

**Rejected: plain words everywhere, including the CLI.** `apply` and `undo` are
the words every tool of this shape uses; renaming them to be friendly would make
the precise surface imprecise.

### The window can *create* a policy, and still never *edits* one

**Chosen.** A folder with no rules is offered a short list of starting layouts —
photos by date, downloads by type, documents by type. Choosing one writes
`.tungstate/policy.toml` into the folder and shows what it would do.

**Why this does not contradict slice 5b.** 5b decided that *editing* a policy
stays in a text editor, because the file lives in the folder, is meant to be
committed, and a form cannot represent everything the parser accepts. All of
that still holds. Writing a *starting file* is scaffolding, not editing —
DESIGN §8 already specifies `tungstate init --template <name>` and §7 already
reserves `policies/` for the templates. The window is getting the same feature
the command line was always going to have.

After it is written, the window shows the file read-only and says where it is.
Changing it means opening it in an editor, and the window says so.

**Rejected: a visual rule builder.** Rejected in 5b and still rejected. The
reason has not changed and the templates make it less tempting, not more.

### A folder is a saved thing, which retires a deferral from slice 6

**Chosen.** Migration v8 adds a `folders` table, and `tungstate folder add` stops
being a stub.

**Why now, when slice 6 deliberately said not yet.** The slice 6 brief refused a
registry because *"a registry is a list, and a list needs an iterator to be
worth having"* — and the only thing that iterated folders was the daemon, three
slices out. That reasoning was correct and this slice is what retires it: **a
window is an iterator.** It cannot walk up from a working directory, because it
has no working directory; it has to show you the folders you have.

The command line gains `folder add`, `folder list` and `folder remove` in the
same breath, because the CLI is the reference surface and must not be able to do
less than the window.

### Before and after, side by side

**Chosen.** The preview is two trees: the folder as it is, and the folder as it
would be, with what moves marked in both.

**Why.** The question someone actually has is *what will my folder look like*,
and a list of `from → to` lines makes them assemble the answer themselves. The
app already has a dual-pane idiom for exactly this shape of question, so the
screen is one people have already met.

`Plan::apply_to` makes it nearly free: the "after" tree is the snapshot the
planner already computes on paper to check that the plan settles. The window is
drawing a value the engine was producing anyway.

**Rejected: a list of moves.** Kept as the detail view behind a toggle, because
it is exact and it is what the command line prints — useful when the two trees
are not enough.

### Applying from the window asks once, and always offers the way back

**Chosen.** The **Tidy up** button summarises what will happen in a sentence and
asks for confirmation. The blast-radius limit is shown as a warning in the
preview rather than as a second refusal, because the window already made someone
read the change and click.

**Why that differs from the CLI.** `--yes` exists because a command can be typed
without reading the plan, or run from a script. A window that has just drawn you
two trees has already had the conversation `--yes` exists to force. The same
number, `Blast::LIMIT_FILES`, is shown either way — what changes is that the
window shows it *before* the button rather than refusing after it.

A policy that does not settle is still a hard refusal on both surfaces, because
that one is a bug in the rules rather than a judgement about scale.

---

## What ships

1. **Journal** — migration v8: a `folders` table (root, added, name), with
   `add_folder`, `folders`, `folder_by_root`, `remove_folder`.
2. **Starter layouts** — `policies/*.toml` at the workspace root, compiled in
   with `include_str!`, each with a one-line description for the picker.
3. **CLI** — `tungstate folder add|list|remove`, and `tungstate init
   [--template NAME] [PATH]` writing a starting policy.
4. **GUI commands** — `folders`, `add_folder`, `remove_folder`, `templates`,
   `write_policy`, `folder_preview`, `folder_tidy`, `folder_undo`,
   `policy_text`.
5. **A Folders tab** — the folder list, the starting-layout picker for a folder
   with no rules, the before-and-after preview, **Tidy up**, **Put it back**,
   and the read-only rules view with its diagnostic.

---

## Acceptance criteria

- `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`,
  `cargo test --locked` clean, `npm run build` clean; CI green on three
  platforms.
- A folder added in the window is listed by `tungstate folder list`, and the
  reverse.
- A folder with no rules offers layouts; choosing one writes the file and the
  preview appears without another click.
- The two trees agree with `tungstate plan` on the same folder.
- **Tidy up** does what the preview showed; **Put it back** restores the folder
  exactly, checked by comparing the tree before and after.
- A policy that does not settle is refused in the window, with the same sentence
  the command line uses.
- A broken policy draws the same diagnostic the command line draws, on the same
  line.
- **By hand, and this is the real bar:** a messy folder tidied from the window
  with no terminal open, then put back — and every screen looked at, because
  three of the last four screen bugs in this project shipped green.
  **Met on macOS**: all twelve states seen, eleven defects found and fixed —
  five of which no test would have caught, including the window reporting
  "moved 9 file(s)" for five moved files. See §7 of the tour. Linux and Windows
  are green in CI but have still not been looked at.

## Tests

**Journal** — the folders table round-trips; a root is not added twice; removing
one leaves its plans intact, so `log` still resolves.

**Templates** — every compiled-in layout parses, produces no warnings, and
settles on a generated folder. That last one is the property test from slice 6
pointed at the templates: a starter layout that churns would be the worst
possible first impression.

**GUI** — the pure functions, as this crate's convention has it: the preview
view carries both trees, the template list is non-empty and each entry has a
description, and the refusals map to the right message.

**CLI** — `folder add|list|remove` and `init --template`, with the snapshot for
`--help` re-read rather than blindly accepted.

## Out of scope

- Editing a policy in the window. Decided in 5b, unchanged.
- Duplicates — slice 8. The preview says nothing about them yet.
- Watching a folder, or tidying without being asked — slice 9.
- Governing a remote folder — slice 13.
- Per-directory mode overrides. `[folder].mode` is read and shown; a folder that
  enforces `Photos/` and only suggests for `Projects/` waits for the daemon.
