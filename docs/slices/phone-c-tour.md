# Phone slice C tour: Files first

The phone app had no way to see or use the phone's own storage, and Free up
chose files by itself. This slice makes it a file manager first, after ES File
Explorer, in tungstate's retro look. It is built to the mockups approved on
2026-11-03 (`docs/mobile/approved-files/`).

What changed, by layer:

1. **Plugin** (`crates/tauri-plugin-photos`): All files access, the category
   tiles' counts and files, Share, and Open in another app.
2. **Engine side** (`crates/tungstate-gui/src`): `volumes.rs` (how full each
   volume is), `fileops.rs` (New folder, Rename, Delete, each journaled), and
   Free up taking the person's unticks and their chosen folder.
3. **Screens** (`ui/src/phone`): Files home, the file window, the clipboard,
   two windows, the Moving screen, Free up's findings, Connections.

Copy and Move are not new engine code. They are `start_transfer`, the same
call the desktop's two panes make, so a phone Move is the desktop's safe move:
send, read back, compare, then remove the original.

---

## 1. Plugin: four commands the screen may call

Slice B's rule was that the window may count and ask, and only Rust may list,
copy out and remove photos. That holds. The new commands are for files the
person is looking at, by their real path:

| Command | What it does |
| --- | --- |
| `files_access` / `ask_files` | All files access, a settings page on Android 11+ |
| `categories` | One pass over Android's file index, a tally per tile |
| `category_files` | Every file in one tile, newest first |
| `share` / `open` | The system share sheet, or the app for that kind |

Each one is a Rust `#[command]` that forwards to Kotlin, plus a line in
`build.rs` and `permissions/default.toml`. Tauri builds the permission files
from `build.rs`. A command missing from it is refused at run time, not at
compile time, so the list is the thing to check.

**Why the index, not a walk.** `categories` asks `MediaStore.Files` once and
sorts every row into tiles. Walking a phone's 100k files would take seconds.
With All files access the index lists every file, not only photos and music.

**Android 10.** Its rules differ: write storage instead of All files access,
and `requestLegacyExternalStorage` on the app, without which files can be
read by path but not moved.

## 2. `volumes.rs`: statvfs without `unsafe`

The workspace forbids `unsafe`. `libc::statvfs` would need it; `rustix`,
already a dependency, wraps the same call safely:

```rust
let stat = rustix::fs::statvfs(path).ok()?;
Some((stat.f_blocks.saturating_mul(stat.f_frsize), stat.f_bavail.saturating_mul(stat.f_frsize)))
```

`f_bavail` rather than `f_bfree`: it leaves out the blocks kept for the
system, which an app cannot write to anyway. `saturating_mul` caps instead of
overflowing.

SD cards are every folder under `/storage` except `emulated` and `self`.
`removable_under` is only called on Android, so on desktop it would be dead
code. `#[cfg_attr(not(target_os = "android"), allow(dead_code))]` quiets the
warning there and keeps the function, and its test, on every platform.

## 3. `fileops.rs`: small steps, each journaled

New folder, Rename and Delete are single steps, not transfers. Each one still
writes its intent to the journal before it acts, and its outcome after:

```rust
fn step(journal: &Journal, op: &NewOp, work: impl FnOnce() -> tungstate_backend::Result<()>) -> Result<(), String>
```

`impl FnOnce()` takes any closure that is called at most once. The step
journals, runs it, and records how it went, so History shows a phone delete
as it shows a transfer.

Two choices a reviewer might question:

- **Rename refuses a name already taken.** The backend's `rename` replaces
  the target without a word, which is right for the engine's atomic writes
  and wrong for a person renaming a file. A case-only rename is let through.
- **Delete is for good.** There is no bin on a phone or a connection, so the
  sheet says so in words before it acts.

Every path is split into its folder and one name, and the backend is opened
on the folder. The backend's own path rules then keep each step inside it.

These are `#[tauri::command(async)]`: over a connection each is a network
round trip, which must not block the window's thread.

## 4. Free up takes unticks, never picks

`Start` gained `left_out: Vec<String>` with `#[serde(default)]`, so a window
that does not send it still works. That's backward compatible across a rolling
update. The run lists the library again itself and removes the left-out ids
from that list:

```rust
assets.retain(|a| !years.contains(&a.year) && (favourites || !a.favourite) && !ids.contains(a.id.as_str()));
```

So the window can only narrow what the filter found. An id that is not in the
library adds nothing, and the test
`photos_unticked_in_the_findings_stay_and_unknown_ids_add_nothing` holds it
to that.

`free_up_findings` returns the list for showing. The chosen folder from
window 2 (`Area51:Photos/Phone`) goes in as `connection` and `folder`. The
existing `folder()` check already accepted nested paths.

## 5. Screens

**One window store, two windows** (`state/useWindows.ts`). Each window holds
where it is, its sort, and what is ticked. Ticking anything makes it
`selecting`, and the app frame then swaps the tab bar for Copy · Move · Share
· More · Delete. At most two windows: a phone shows one, and the second
exists for a copy or a move between two places.

**The clipboard** (`state/useClipboard.ts`) holds paths and an op. "Move
here" turns them into one transfer with one leg per source folder
(`lib/paths.ts: legsFor`), because the engine moves names out of one folder
at a time and a category's files span many. Phone transfers use `readback`
and `rename` on a clash, so a run never stops to ask about a clash. It does
still ask about an identical copy already there, and the app frame shows a
sheet for that.

Free up uses the same bar. Its clip carries a `land` function in place of
paths, which starts the Free up run in the folder chosen.

**The Moving screen** follows the desktop's transfer ledger
(`state/useTransfer.ts`), so sent, checked and removed are counted from each
file's own events (`lib/progress.ts: tally`). One trap, worth knowing: when a
run ends, the ledger hands over the very array it was filling, and a Vue
`computed` that returns the same reference does not tell anything that
depends on it. The counts stayed at their mid-run values until the screen
copied the array at the end.

Pause and Stop both finish the file in hand. After a pause, "Move the rest"
sends again only what was not checked (`lib/progress.ts: unfinished`). Moved
files are gone from their folders, so nothing is sent twice.

**Thousands of findings.** Each year shows its first 60 rows until asked for
the rest; ticking a year still covers all of it. A phone's web view slows
down badly with ten thousand buttons.

## 6. What is not in this slice

- **Thumbnails.** Rows and grid tiles show the kind's mark, not the photo.
  Real thumbnails need the asset protocol (or a small picture server) scoped
  to shared storage, which is its own decision.
- **Share and Open from an SD card.** The FileProvider covers internal
  storage only (`file_paths.xml`).
- **Tapping a file on a connection** ticks it, instead of downloading and
  opening it.
- **Found on your network** is the next slice (discovery).
- **iPhone.** Files on iOS is a different model (the app's own container and
  the document picker). Not built here.

## Checks

- `npm run build` (CSS check, `vue-tsc`, Vite) and `npm test`: 64 pass,
  including `phone-paths` and `phone-clipboard`.
- `cargo test -p tungstate-gui`: 87 pass, including `volumes` (3),
  `fileops` (5) and the Free up untick test.
- Harness shots of every state: `scripts/shoot-phone.sh` with `?open=`,
  `?second=`, `?hold=`, `?move=run|done|paused`, `?identical`, `?view=grid`,
  `?files=ask` (see the top of `src/dev/phone-mock.ts`).
