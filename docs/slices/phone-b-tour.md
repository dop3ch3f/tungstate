# Phone slice B tour: Free up space

What this slice did, in three steps, each its own commit:

1. **Read the library.** A new plugin, `crates/tauri-plugin-photos`, asks
   for photo access through the system's own prompt and counts photos and
   videos by year. Free up space and Home show what could be freed.
2. **Send and check.** Free copies each photo out of the library, sends it
   to a connection through the existing transfer engine, and checks it
   there. New: `crates/tungstate-gui/src/freeup.rs`.
3. **Clear.** After the run, only the photos that were checked are removed
   from the phone, through Android's own delete prompt.

iPhone is step 4 and waits for Xcode. Until then the plugin's Swift side
says "not on iPhone yet" rather than answering with nothing.

---

## 1. A Tauri plugin: Rust in front, Kotlin behind

A phone's photo library is only reachable through the phone's own APIs
(MediaStore on Android, PhotoKit on iPhone), so the work is in Kotlin and
Swift and the Rust crate forwards to it.

The crate builds differently per platform with `#[cfg]` (slice A, §1):

```rust
#[cfg(desktop)]
pub use desktop::Photos;
#[cfg(mobile)]
pub use mobile::Photos;
```

Two structs named `Photos` with the same methods, and each build sees one.
On desktop every method returns `Error::NoLibrary`, so the app is built the
same way everywhere and the window just shows nothing to free.

On a phone each method sends a named command to Kotlin and waits:

```rust
pub fn list(&self, filter: Filter) -> Result<Vec<Asset>> {
    Ok(self.0.run_mobile_plugin::<Assets>("list", filter)?.assets)
}
```

`run_mobile_plugin::<Assets>` is a generic function, with the type to decode
the answer into given in the turbofish (`::<...>`). Kotlin answers with JSON;
serde turns it into `Assets`. The `?` turns a failure into this crate's
`Error` through a `#[from]` conversion in `error.rs`.

**Who may call what.** The window may only count and ask for access
(`permissions/default.toml`). Listing, copying out and removing are plain
Rust methods with no `#[command]`, so only the app's own Rust reaches them.
That matters for removing: the window can say "clear what was checked", but
it can never name a photo to delete.

**Commands arrive on Android's main thread.** Anything slow there freezes
the screen, so `PhotosPlugin.kt` hands counting, listing and copying to one
background worker (`Executors.newSingleThreadExecutor()`).

## 2. A trait as a seam: `Library`

`freeup.rs` needs three things from the phone: list, copy out, and later
remove. It asks for them through a trait:

```rust
pub(crate) trait Library {
    fn list(&self, filter: &Filter) -> Result<Vec<Asset>, String>;
    fn export(&self, asset: &Asset, to: &Path) -> Result<(), String>;
}

impl<R: Runtime> Library for Photos<R> { ... }
```

A trait is a set of methods a type promises to have. The plugin's `Photos`
implements it on a phone; the tests implement it with `FakeLibrary`, a few
photos held in memory. So the whole run, from choosing to sending to
deciding what is safe, runs in `cargo test` on a desktop.

`&dyn Library` in `carry`'s signature means "any type that implements
`Library`", picked at run time. That is what lets one function take the
plugin or the fake.

One snag: `Photos` also has its own `list(filter: Filter)`, and Rust picks a
type's own method before a trait's. So the run calls `Library::list(photos,
&filter)` to say which one it means.

Removing is not in the trait. It happens once, after the run, from its own
command, and only through the plugin; nothing in the run needs it.

## 3. Copy out, then send

A library is not a folder the engine can walk, so each batch of photos is
copied into the app's cache, laid out as it will land:

```
cache/freeup/2025/Photos/IMG_0506.jpg
cache/freeup/2025/Videos/VID_0704.mp4
cache/freeup/2025/Screenshots/Screenshot_0901.png
```

and the ordinary engine moves that folder to `<connection>:Phone`:

```rust
let sent = Transfer::new(run.link, &source, run.destination, run.journal,
                         &mut FixedResolver(ConflictAction::Rename), &mut recorder)
    .cancellable(Arc::clone(run.stop))
    .run_selection(&sending);
```

The link says `SourcePolicy::Delete`, so the engine deletes each copied-out
file once it is verified at the far end. The cache never holds more than a
batch (500 MB, or one video if it is bigger).

The layout is decided in `layout()`. Two photos can share a name in one
folder, and SMB ignores case, so the second gets a short tag from its id
(`IMG_0001-3f2a9c1b.JPG`). A tag, not a counter, so the same photo gets the
same name on every run whatever else was chosen. `tag()` is FNV-1a, a
four-line hash, because std's own hasher may change between Rust releases.

Names come from the library, so `clean()` treats them as untrusted: no
slashes, nothing SMB refuses, no `..`.

## 4. What counts as safe

The engine reports every file through the `Progress` trait. `Recorder`
implements it and writes down each outcome. A photo is safe only if:

```rust
fn safe(outcome: FileOutcome) -> bool {
    matches!(outcome, FileOutcome::Transferred | FileOutcome::AlreadyPresent(Original::Removed))
}
```

`matches!` is a macro that is true when a value fits a pattern. `|` means
either pattern. Both mean the engine checked the copy and then deleted the
copied-out file, which is its own signal that it was satisfied. Anything
else (failed, skipped, a copy it kept) is not proof, and the photo stays.

The test `a_photo_whose_copy_failed_its_check_stays` sends through a
backend that corrupts one file. Read back catches it, and that photo is not
in the safe list. Making `safe()` return `true` for everything makes the
test fail, so it does guard this.

**Why the safe list stays in Rust.** `FreeUp::checked` holds the ids. The
done event sends the window counts, never ids (`#[serde(skip)]` on
`Ran::safe`). The clear command reads the ids from that state.

## 5. A rerun needs no bookkeeping

If the phone dies mid-run, nothing records which photos made it. A rerun
copies them out again, and the engine finds the same name with the same
bytes already there. It reads both copies to be sure, and reports
`AlreadyPresent(Original::Removed)`, which counts as safe. Nothing is sent
twice (`a_rerun_counts_what_already_arrived_as_checked_without_sending_it_twice`).

The one thing a dead run leaves is a half-written temporary file on the
connection. The engine cleans those up by *link*: the next run of the same
link finds the unfinished operation in the journal and removes its partial.
So `link_for` reuses one link per connection, folder and check level
(`free-up Area51:Phone readback`) rather than making a fresh one each time.

## 6. One run at a time: the gate and the queue

Transfers and syncs already take `App::gate` before deciding to start, so
two of them can't both see the other idle. Free up does the same, and while
it holds the gate it claims the transfer queue:

```rust
let _gate = lock(&state.gate);
if state.syncing.queue.is_running() { return Err(...); }
if !state.queue.claim() { return Err(...); }
state.freeup.running.store(true, Ordering::SeqCst);
```

Claiming the queue means syncs, synced folders and the updater all see a
transfer running, with no changes to their code. The one gap: a transfer
pressed meanwhile would be queued behind a worker that never takes it, so
`spawn_run` now refuses while `freeup.running()` is set.

`Guard` releases everything when the run's thread ends, including by a
panic, because `Drop` runs during unwinding. It also lets the screen sleep
again.

`_gate` with a leading underscore keeps the lock held to the end of the
block. A bare `_` would drop it at once.

## 7. Clearing: once, on screen, by the system's prompt

On Android 11 and later, `MediaStore.createDeleteRequest` builds a system
prompt ("Allow Tungstate to delete 6 items?"). If the person agrees,
Android deletes them for good: no trash. Kotlin then counts how many of the
ids still exist, rather than assuming.

`free_up_clear` is `#[tauri::command(async)]`. A plain command runs on the
main thread, and this one waits for the person's answer, which needs the
main thread to draw the prompt. That would deadlock.

The window calls it only while the app is on screen (`onScreen` in
`useFreeUp.ts`): a run that ends with the phone locked clears when the
person comes back. Android 10 and earlier cannot be asked at all, and the
done screen tells the person to delete them in their gallery.

**Location data.** Android strips where a photo was taken from copies
unless the app holds `ACCESS_MEDIA_LOCATION`. A NAS copy without it is not
worth deleting the original for, so the plugin asks for it with the photo
permission (it has no prompt of its own) and refuses to copy out a photo
without it.

## 8. The screens

- **Free up** (`FreeUpScreen.vue`): kind, age and years, favourites apart,
  "To Area51 › Phone", and the check level in the filter sheet. Read back is
  the default. Each level states its cost: read back is slower; size only
  would miss a copy damaged on the way.
- **Running** (`FreeingScreen.vue`): the approved mockup, with the tabs
  hidden. GB sent, a 20-block bar, "1,448 of 3,214 checked on Area51", time
  left once there is a pace to go by. Pause and Stop both finish the photo
  in hand; Pause keeps the choice so Resume sends the rest.
- **Done**: how many are safe and where; the system prompt; then "Freed on
  this phone" and what deleting means on that platform.

The lock note says what is true today: keep the app open. Sending while
locked needs an Android foreground service and iOS background uploads,
which is a later slice.

Harness: `?run=done|kept|lost`, `?clear=no|old|wait` and `?android` in
`phone.html` show every state without a phone.

## 9. Checked on Android

On the `tungstate_test` emulator (Android 14) against the Samba server in
Docker, with eight test files pushed into the camera and screenshot folders:

- **Counting.** The system prompt asked for photo access; after "Allow
  all", Free up showed 2025: 4 items, 2024: 2 items, favourites 432 KB
  apart, 17 MB in all. That matches what was pushed. The photo from this
  month was left out by "older than 3 months". The location permission
  came with the photo grant, with no extra prompt.
- **Sending.** Six files landed in `Phone/2024/Photos`, `2025/Photos`,
  `2025/Videos` and `2025/Screenshots`, each byte-identical to its original
  (md5 compared). The favourite and the recent photo stayed behind.
- **Clearing.** Android asked "Allow Tungstate to delete 6 items?". After
  Allow, MediaStore listed only the two that were never chosen.
- **Deny.** All seven stayed on the phone, and the done screen offered to
  ask again.
- **Killed mid-send.** Force-stopped 25 seconds into a 106 MB video: a 37 MB
  `.part` file was left on the share and nothing was removed from the phone.
  After reopening and pressing Free again, the partial was gone, the video
  arrived whole (md5 matched), the six already there counted as checked
  without being sent twice, and clearing removed all seven.
- **Killed at the delete prompt.** Nothing was removed.

Two bugs found this way and fixed: Kotlin's bare `resolve()` answers
`null`, which the Rust side first tried to read as an object (now `()`).
And a run where every photo failed said "Stopped"; it now says nothing
could be sent, and why.

**Not handled yet.** Docker Desktop was paused during one run. The SMB
connection hung for about 35 minutes with "Getting ready…" on screen, then
carried on by itself when Docker came back. Nothing was lost, but a NAS that
accepts a connection and then goes silent needs a time limit. That belongs
in the SMB backend, not here.

## 10. Building

As in slice A (§8). One new trap: the debug APK is 1.1 GB because of Rust's
debug symbols. Replacing it in place on the emulator needs room for both
copies and failed with `INSTALL_FAILED_INSUFFICIENT_STORAGE`.
`adb shell cmd package uninstall -k com.ifeanyiibekie.tungstate`, then
`adb install`, keeps the app's data (connections, photo permission).
