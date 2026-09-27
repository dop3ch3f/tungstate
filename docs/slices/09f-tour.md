# Slice 9f tour: updates, dark mode, and buttons that say they are working

Three things you asked for between 9e and the daemon:

- The installed app updates itself from tagged releases, and asks first.
- Dark mode, as a set of themes.
- Every button that waits on the engine shows it is waiting and cannot be
  pressed twice.

The 9e tour is [09e-tour.md](09e-tour.md).

---

## 1. A plugin is a step in the builder

Tauri's updater and process-restart support are plugins, crates that add
commands to the app. They join the same chain every other part of the app is
built with, in `main.rs`:

```rust
.plugin(tauri_plugin_dialog::init())
.plugin(tauri_plugin_updater::Builder::new().build())
.plugin(tauri_plugin_process::init())
```

`tauri::Builder` is the builder pattern. Each call takes the builder by value
and hands it back with one more thing configured. That's why the calls chain,
and why no `mut` variable is needed. Nothing runs until `.run(...)` at the end.

The window calls the plugins from TypeScript (`check()`, `update.download()`,
`update.install()`, `relaunch()`). Under the hood those are `invoke`s like
ours, with names such as `plugin:updater|check`.

## 2. Capabilities are the permission list

`capabilities/default.json` lists what the window may call. Anything not in
the list is refused, which is the lesson of alpha.1: an empty list meant the
window could not even listen for events. This slice adds three entries:

- `core:window:allow-set-theme`, so the title bar follows the theme;
- `updater:default`, to check for and download an update;
- `process:allow-restart`, to start the new version.

Our own commands are allowed by being in `generate_handler!`. The plugins'
commands have to be granted here.

## 3. Two kinds of signing

This app is still not signed by Apple or Microsoft: that needs paid
certificates, and the release notes explain the warnings. **Updates are signed
anyway**, with a key of our own. The updater refuses any download whose
signature does not match the public key built into the app
(`tauri.conf.json`, `plugins.updater.pubkey`). A tampered file on GitHub, or
one swapped in on the way, is never installed.

The private key is in your macOS Keychain (service `tungstate-updater`) and in
the repository's secrets, where the release workflow signs with it. **If it is
lost, installed copies cannot accept new updates** and everyone reinstalls by
hand once. That's why there are two copies.

## 4. How a release reaches an installed app

- A tag builds the installers as before. With `createUpdaterArtifacts` on,
  each platform also gets an update file and its `.sig`:
  - `Tungstate_macos-arm64.app.tar.gz` and its x86_64 twin (renamed on the
    runner, because both Macs call theirs `Tungstate.app.tar.gz`);
  - the Windows `-setup.exe`;
  - the `.AppImage`.
- `scripts/updater-json.mjs` writes `latest.json`: the version, the tag's
  message as the notes, and one URL and signature per platform. It refuses to
  write one with a platform missing, so a release can never tell half its
  users there is nothing new.
- The app reads
  `https://github.com/dop3ch3f/tungstate/releases/latest/download/latest.json`.
  GitHub's `latest` skips prereleases, so **tagged releases are now full
  releases**, alpha or not. `rolling` stays a prerelease and never publishes
  a `latest.json`, so installed apps only follow numbered versions, as you
  chose.
- The `.deb` cannot update itself; the notes say so.

## 5. The restart waits, and nothing starts behind it

`useUpdate.apply` downloads, then asks whether it is safe to restart, then
installs and relaunches. The asking has two halves:

- **The window's own work:** a tidy in progress, or duplicates being cleared.
- **The engine's:** a new command, under the same gate every run takes:

```rust
#[tauri::command]
fn ready_to_restart(state: State<'_, App>) -> Option<Blocker> {
    let _gate = lock(&state.gate);
    let busy = blocker(&state.queue, &state.syncing.queue);
    if busy.is_none() {
        state.following.halt();
    }
    busy
}
```

`Option<Blocker>` crosses to TypeScript as `null` or `"transfer"` / `"sync"`
(`#[serde(rename_all = "kebab-case")]` sets the spelling). It is data, and the
window words it: "Waiting for the transfer to finish."

The check and the halt happen while the gate is held, so a sync kept in step
cannot claim the queue between "nothing is running" and "stop following". If
the install then fails, `not_restarting` starts following again.

`blocker` is **generic**:

```rust
fn blocker<A, B>(transfers: &RunQueue<A>, syncs: &RunQueue<B>) -> Option<Blocker>
```

The transfer queue holds transfer jobs and the sync queue holds sync jobs.
`<A, B>` says "any two item types", which is also what lets the test hand it
two queues of `u8` instead of building a whole app.

## 6. Dark mode

- **Where the choice lives.** It is kept in the window's own storage
  (`localStorage`), not the journal. It is a preference about this window, and
  it has to be known before the first paint, so the window never flashes the
  wrong colours while a command answers. `lib/theme.ts` is a pure function
  from (choice, system is dark) to what goes on `<html>`, and is tested.
- **The choices.** Match the system (the default), retro, retro dark,
  graphite and paper. On "match the system", the window switches live when
  the Mac does.
- **Retro dark is retro's shapes with another palette.** `data-theme="retro"`
  stays and `data-tone="dark"` is added, so every rule that draws retro's
  outlines and shadows applies unchanged. Only colours change, in
  `tokens.css`.
- **What the palette needed.** Three new tokens let a dark palette rank
  things the light one did not have to:
  - `--rule`: lines between rows, quieter than frames;
  - `--chosen`: the fill of whatever is chosen;
  - `--chosen-edge`: the outline of a chosen card, in the section's colour.

  In the existing themes they equal what was drawn before, which the
  screenshots of all four themes confirmed.
- **How the look was chosen.** Four rounds with an outside critic stalled at
  6/10, and the critics disagreed with each other. One asked for bright
  cream chips for the chosen item; the next said they glared. You chose
  between three directions side by side: B, bold cream outlines, a black slab
  shadow, and a dark raised chosen state.

## 7. Buttons that say they are working

- `Button` gains a `busy` prop. A busy button takes no clicks, keeps its
  colour, label and width, and pulses. Busy is deliberately not the same as
  disabled, which greys the button out.
- `state/useBusy.ts` is the **single-flight guard** behind it, lifted from
  Organize's `run`. `run(key, work)` refuses a second call under a key that
  is already running, lets the button draw its busy state first, and catches
  errors into one slot. Keys let each row wait on its own: every Put back in
  a list, every link's Run.
- `lib/latest.ts` covers the other shape: requests that may overlap, where
  only the newest answer may land. Conflict previews, History's Look and the
  file pane's reads use it.

## 8. What checking found

Four bugs, each fixed in its own commit with a test:

- **A double click on "Stop after these files" became "Stop now".** The
  harsh stop appears exactly where the gentle one was clicked. Both run
  screens now share `StopPair`, whose "Stop now" ignores clicks for 600 ms
  after it appears.
- **Quick answers to conflicts showed stale numbers.** Each answer previews
  again, and whichever preview came back last was shown. The run then failed
  safe, refused as changed, but the screen was wrong until then.
- **Opening a folder while another loaded paired it with the other's
  preview.** Tidy up would then have acted on a folder whose plan nobody saw.
  The folder now changes only once its own preview is in.
- **A link's Run queued the pair twice** on a double click. The busy guard
  covers it.

The tests for the window's state modules now run against a fake engine
(`scripts/fake-engine.mjs`, which stands behind Tauri's `invoke`). A small
resolve hook teaches Node the extensionless imports Vite accepts.

## 9. An update, for real, on this Mac

Nothing was published for this check:

1. I built a signed 0.0.2 app and served its update and a `latest.json` from
   `127.0.0.1`.
2. I built 0.0.1 pointed at that server and ran it against a throwaway
   journal.
3. Within seconds the sidebar said "Version 0.0.2 is ready".
4. The sheet showed the notes. Update and restart downloaded 11 MB,
   installed, and the app reopened as 0.0.2. Settings then said "It is the
   newest".

One thing it found: run from `/tmp`, the updater refuses, because `/tmp` is a
symlink on macOS and Tauri will not replace a program reached through one.
From `/private/tmp`, or `/Applications`, it works.

## 9b. The window, driven by hand, for 9d and 9e

This used a throwaway journal and two folders standing for the laptop and the
NAS, with a sync made from the terminal and set to keep in step:

- **The opening run** carried the file already there.
- **A save** arrived in one run. The run showed up in the list by itself, and
  the screen stayed where it was.
- **A deletion** held the sync. The yellow notice and "A sync is waiting for
  you" on the sidebar both appeared. "Look at it" showed the removal and the
  blast limit asking for a yes; ticked and run, the file was set aside.
- **Keeping in step resumed** on its own: the next save arrived.

It found one more bug, fixed in its own commit with a test: **a background
run's progress wrote itself into the result screen of the run before**. Its
file and bytes appeared above that run's summary and pushed Put it back down
the page. Progress now counts only while a run somebody pressed is being
watched. The fake engine behind the tests can now send events, which is how
the test replays it.

## 10. What is checked

- **Rust:** `an_update_waits_while_a_transfer_or_a_sync_is_writing`.
- **The window's tests** (`npm test`, 22 in all), including:
  - an update waiting for a transfer, then installing and restarting;
  - a failed install starting syncs again;
  - `latest.json` for every platform, and refused when one is unsigned;
  - the theme resolving;
  - the busy guard;
  - the newest-answer guard;
  - the stop holdoff;
  - the folder-opening fix.
- **Screens** in the headless harness, in all four themes, plus the busy and
  update scenes.

## What is still not verified

- **A real published update.** alpha.4 is the first version with the updater
  in it, so it has to be installed by hand once. The first real update will
  be alpha.4 to alpha.5.
- **Windows and Linux updates** have not been run by hand; CI builds and
  signs them.
- **The restart waiting on a real transfer** was checked against the fake
  engine, not in the window.
- **A day of use.** The window was driven through one sync's life, not left
  open through an afternoon of exports.
