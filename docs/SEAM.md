# The seam

**What this is:** the contract between the engine and any window built on it.
Written down because the window is going to be redesigned, and a redesign is
only cheap if it needs no Rust.

**The test for "locked":** a new front end can be built against this document
without changing a single line in `crates/`. If a redesign finds itself
needing a Rust change, that is a bug in this seam and belongs here rather than
in the new UI.

---

## Why there is a seam at all

The engine is roughly 21,500 lines across six crates, with 467 tests. The
window is roughly 3,700 lines of Vue, TypeScript and CSS. **A redesign replaces
the second number and touches none of the first.** Everything that moves a
file, decides where it belongs, or remembers what happened lives on the engine
side and is already covered by tests.

The seam itself is 50 `#[tauri::command]` functions in
`crates/tungstate-gui/src/main.rs`.

## The two rules that make it lockable

### 1. Results cross as data, not as sentences

A command returns counts, names and structures. It does not return prose.

This was not true until slice 7e. `tidy_folder` used to return
`"moved 5 file(s)"`, and a window that receives a sentence can only print it —
there is no way to show the **5** large, to put an undo beside it, or to word it
differently, without changing Rust. A screen that cannot restyle its own
results is a screen that cannot be redesigned.

So:

| Command | Returns |
|---|---|
| `tidy_folder` | `{ moved, skipped, failed, already_tidy }` |
| `put_back` | `{ files }` |
| `folder_preview` | `PreviewView` — trees, moves, counts, blast radius |
| `learn_folder` | `{ levels, explains, of, loose, as_is, improved, suggestions }` |
| `compare_folder` | `Outcome[]` — per layout: counts, settles, one example move |

`already_tidy` is a separate flag rather than `moved: 0`, because "there was
nothing to do" and "it moved nothing" are different sentences and only the
window knows which one it wants to say.

Three commands return a **name** rather than prose, which is data and is fine:
`reset_storage` and `restore_archive` return the archive the previous state was
put under. `rules_text`
returns a policy file, which is text because it *is* text.

### 2. The command line and the window compute the same answer

Where both surfaces answer the same question, the answering lives in
`tungstate-core` and both call it. Otherwise they drift, and the drift is
invisible until somebody compares two screens.

- `tungstate_core::learn` — read a folder's own shape (`folder learn`,
  `learn_folder`)
- `tungstate_core::compare` — what each layout would do (`folder compare`,
  `compare_folder`)

Slice 7e moved the CLI's private copy of the comparison into core for exactly
this reason, and the move immediately surfaced a bug neither surface had shown
on its own.

## What the window can ask for

Grouped by the question being asked rather than by call name.

**Folders** — `governed`, `govern_folder`, `forget_folder`, `layouts`,
`give_rules`, `rules_text`, `folder_preview`, `tidy_folder`, `put_back`,
`learn_folder`, `compare_folder`, `pick_folder`, `past_tidies`.

**Duplicates** — `find_duplicates` (any folder or `connection:folder`, emits
`dupes://progress`; `similar` also looks for files that are nearly the same,
which is local-only), `stop_finding_duplicates`, `clear_duplicates`,
`undo_duplicates`, `recent_scans`, `past_cleanups`, `duplicate_action`,
`set_duplicate_action`.
The scan needs no governed folder: finding duplicates has nothing to do with
rules. That is also why the undo is here rather than shared with the folder
half, whose `put_back` walks up to a policy file and refuses without one.

`clear_duplicates` takes **copies, not groups**, because every checkbox on the
screen is its own copy. It refuses any set of them that would leave a group
with nothing, so *every group keeps one copy* is enforced by the engine rather
than arranged by the window (slice 8d).

Small pictures do not travel through a command at all: they are served over a
`thumb` URI scheme from the app's cache directory, because fifty photographs as
base64 is most of a megabyte of JSON per redraw.

**Watching** — `watch_state`, `set_watching`, and the `watch://noticed`
event. The switch is remembered in the journal. `watch_state` returns what the
watcher has done since the window opened, and the window reads it back on
every event rather than keeping its own list, because which notice replaces
which is a rule the engine owns: a folder that is still waiting shows one line,
not one per sweep (slice 9).

**History and search** — `recent`, `history`, `whereis`, `quarantined`.
`history` and `whereis` accept a path spelled any way a person might type it
(slice 7d) and `whereis` also takes a BLAKE3 digest.

**Transfers** — `browse`, `places`, `preview_transfer`, `start_transfer`,
`cancel_run`, `stop_now`, `list_links`, `create_link`, `run_link`,
`remove_link`, `preview_link`, `set_at_once`.

Transfers are a queue, like downloads (polish pass, 2026-09-27). One runs and
the rest wait in the order they were asked for. `start_transfer`, `run_link`
and `resume_interrupted` return `Accepted { started, waiting, job }`, where
`waiting` counts the transfers behind the running one, this one included.
`transfer_queue` returns `{ running, waiting }` as `JobView`s (route, move or
copy, exchange, and `files`/`bytes` once counted from the source alone), and
`remove_from_queue(job)` takes a waiting one out, answering `false` if it had
already started. `transfer://queue` carries the same listing on every change.
`transfer://job` announces each transfer as it starts, and every file event
after it (`began`, `planned`, `started`, `advanced`, `checking`, `finished`)
is that transfer's until its `transfer://done` (the totals plus `job`) or
`transfer://error` (`{ job, message }`). A transfer that fails, or whose far
side disappears, is its own: the next one still runs. Stop is the exception
and clears the queue.

**Syncs** — `list_syncs`, `make_sync`, `change_sync`, `add_sync_member`,
`remove_sync_member`, `remove_sync`, `preview_sync`, `run_sync`, `stop_sync`,
`past_syncs`, `put_back_sync` (slice 9d). Making and changing one is checked
by `tungstate_sync::setup`, the same code `sync add` and `sync set` use, and a
refusal crosses as data (`{ kind: "overlap", first, second }`), never a
sentence. `preview_sync` returns `SyncPreviewView`: per member, copies
arriving, copies leaving, versions replaced, files taken off, files deleted,
renames and parked conflicts, each its own number and never summed; the files
taken off listed one by one with why; conflicts with each member's version;
"left alone" with a reason kind per path; and the refusals (`hollow`,
`blast`) as data. It also carries a `fingerprint`, which `run_sync` must be
given back: the engine decides again when the run starts and refuses, with
`sync://error` of kind `changed`, a run that would now do something other
than what was shown. Conflicts are answered in the preview, before anything
moves, through `ask.resolve`, and the answers are previewed again so the
numbers on screen are the run's. A run emits the transfer events under
`sync://` (`planned`, `started`, `advanced`, `checking`, `finished`), a
`sync://leg` as each pair of members begins, then `sync://done` with
`SyncRanView` or `sync://error`. Syncs run one after another, and never
beside a transfer. `put_back_sync` returns what came back, including the
members whose deletion was undone too (`revived`).
A sync set to keep in step (`launch: "continuous"`, slice 9e) is run by a loop
the window starts itself: `following_state` returns each member's state
(`watching`, `polling` with `every_secs`, `paused` with `why`) and the syncs
held because a run would remove files, and `sync://following` sends the same
view whenever any of it changes, never once per look. `resume_following`
releases a held sync; the window calls it after a person has run one. Runs
started this way report through the same `sync://done` and `sync://error` as a
pressed Run, and an error that repeats is sent once.

**Connections** — add, list, update, test, remove, set a password.

**Updates** (slice 9f) — the updater and process plugins answer `check`,
`download`, `install` and `restart` themselves. The engine adds one question:
`ready_to_restart` returns `null` when nothing is writing files, and then stops
keeping syncs in step, or `"transfer"` or `"sync"` for what the restart must
wait for. `not_restarting` starts syncs kept in step again when an update
fails after that. The theme is not in the seam: it is the window's own choice,
kept in the window's storage.

**Storage** — `archives`, `reset_storage`, `restore_archive`, `forget_archive`,
`export_storage`, `import_storage`.

## What a redesign must keep

Not style — these are safety properties, and a simpler screen makes them more
important rather than less:

1. **Nothing moves without the person having seen what would move.** The
   preview exists so the button is never a guess.
2. **Everything is reversible, and the way back is visible at the moment it is
   needed** — not buried in a history screen. `PreviewView.undoable` is the
   most recent reorganisation of a folder that can still be put back, offered
   beside the button that caused it. Older ones come from `past_tidies` and
   `past_cleanups`, which list each section's own runs with their plan ids, so
   Organize and Duplicates each show a history where somebody is standing when
   they want to undo something. A plan records what it was for from journal
   v11; older plans are judged by what they did.
3. **A count of files and a count of operations are different numbers.** A plan
   makes and removes directories too. Slice 7b shipped "moved 9 file(s)" for
   five moved files.
4. **"Left alone" is not one thing.** A file may be untouched because no rule
   claimed it, because it is inside its cooldown, or because it changed while
   we looked. A screen that collapses those into one word hides the only
   question worth asking.
5. **Directories removed is as important as files moved.** It is the number
   that says a layout is *replacing* a shape rather than adding one.

## What is deliberately not in the seam yet

- **Progress for a long tidy.** Transfers emit events, and as of slice 8c the
  duplicate scan emits `dupes://progress`; tidying still does not. A folder of
  100,000 files would tidy with no feedback. The pattern is now established
  twice over, so this is a command away rather than a design question.
- **Extended attributes.** macOS records the downloading app in
  `com.apple.quarantine`; `Attributes` has no field for it, so no command can
  expose it.
- **Search at scale.** `whereis` is a scan over the ops table.
  `tungstate-index` exists in the plan and is unbuilt.

Each of those is a known gap with a known shape. None of them blocks a
redesign; all of them would be additions to this seam rather than changes to
it.
