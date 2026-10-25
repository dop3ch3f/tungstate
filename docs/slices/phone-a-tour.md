# Phone slice A tour: the app on phones, its own screens, and Connections

What this slice did:

- The engine and the window build for iPhone and Android (the P0 test, in
  commit bc4bb3e).
- Phones get their own app, built for touch from the approved mockups in
  `docs/mobile/approved/`, rather than the desktop window squeezed small.
- The first working tab is Connections: add a NAS share, FTP, SFTP, WebDAV
  or S3, check it, save it, and browse it a folder at a time.

Free up space, Duplicates and Organise come next. Their tabs say so in plain
words until then.

---

## 1. One codebase, two kinds of device: `#[cfg]`

Rust can leave code out of a build depending on what it is built for. The
attribute `#[cfg(...)]` on an item means "only compile this when the
condition holds":

```rust
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn trash_original(full: PathBuf) -> Result<()> {
    trash::delete(&full).map_err(|source| TransferError::Trash { path: full, source })
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn trash_original(full: PathBuf) -> Result<()> {
    Err(TransferError::NoTrashOnDevice { path: full })
}
```

Two functions with the same name, and each build sees exactly one. Phones
have no Recycle Bin, so on a phone a request to send an original there is
refused with a clear error. It is never quietly turned into a permanent
delete.

`Cargo.toml` can do the same for whole dependencies, under a
`[target.'cfg(...)'.dependencies]` table. That is how the `trash` crate, the
desktop updater and `keyring`'s desktop stores stay off phones.

The window uses Tauri's shorter spellings, `#[cfg(desktop)]` and
`#[cfg(mobile)]`, which mean the same thing.

## 2. Passwords on a phone: `tungstate-secret/src/phone.rs`

`keyring`'s all-in-one `Entry` only knows desktop stores. On phones we name
the store ourselves: iOS's protected keychain, or the Android Keystore. It is
installed once, the first time a password is touched:

```rust
static READY: LazyLock<bool> = LazyLock::new(|| match open() {
    Ok(store) => { keyring_core::set_default_store(store); true }
    Err(_) => false,
});
```

`LazyLock` is a value that is worked out the first time anyone reads it, and
then kept. It is safe across threads, which matters because transfers run on
their own threads. The small `Entry` wrapper below it has the same method
names as `keyring::Entry`, so `lib.rs` reads the same on every device.

Checked on Android: a connection saved, the app force-quit and reopened, and
the share opened with the password read back from the Keystore.

## 3. The window became a library: `main.rs` → `lib.rs`

A phone starts an app by loading it as a library and calling one function.
So the window's code moved to `src/lib.rs`, with a public `run()`, and
`src/main.rs` is now six lines that call it. The attribute

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
```

means "on a phone, also mark this as the entry point". `cfg_attr` applies an
attribute only when the condition holds.

A phone also only learns where its app folders are once Tauri is running.
So on desktop the journal opens before the window, as it always did. On a
phone it opens in `setup`, after `PHONE_DIRS` records the folders.

## 4. Which app opens: `ui/src/main.ts`

```ts
if (onPhone()) {
  void import("./phone/PhoneApp.vue").then(({ default: PhoneApp }) => createApp(PhoneApp).mount("#app"));
} else {
  createApp(App).mount("#app");
}
```

`onPhone()` reads the web view's own name for itself (iPhone, Android),
which is known before anything loads, so desktop start-up waits on nothing.
The phone app is a separate bundle (`PhoneApp-*.js`) that desktop never
downloads.

## 5. The phone app: `ui/src/phone/`

- `PhoneApp.vue`: one screen over a tab bar, with the safe-area spacing for
  the notch, the status bar and the home bar.
- `TabBar.vue`: Home, Free up, Duplicates, Organise, Connections. The current
  tab is white, outlined, with the hard shadow, your rule from desktop.
- `ui/`: `PScreen` (back link, big title, scrolling body, main action pinned
  in thumb reach), `PRow`, `PButton` (its shadow collapses when pressed),
  `PTile`.
- `screens/ConnectionsScreen.vue` and `AddConnection.vue`: the working tab.
- `screens/HomeScreen.vue` shows only what it really knows, your
  connections. The "how much you can free" number needs the photo library,
  and the card says so.

Everything uses the desktop's theme tokens, so retro light and retro dark
both work, and the same engine commands (`ui/src/engine/commands.ts`).

## 6. Four phone traps this slice hit

- **Android keyboards "compose" words.** Vue's `v-model` ignores typing
  until a word is finished, so a filled-in address read as empty and Save
  stayed grey. The phone forms read every input event directly instead.
- **The keyboard covered Check and Save.** Tauri draws Android apps edge to
  edge, and then Android stops shrinking the window for the keyboard.
  `MainActivity.kt` now pads the page by the keyboard's height, and the tab
  bar hides while you type.
- **Slow steps looked frozen.** Saving and opening took about ten seconds in
  the debug build. Buttons now say "Checking…" and "Saving…", and opening
  says "Opening…". A release build should be much faster; it gets measured
  before anything ships.
- **This Mac's Java is an Intel build.** Gradle under Rosetta crawls and
  looks hung. Android builds need `JAVA_HOME` pointed at the arm64 JDK 19.

## 7. Checking phone screens without a phone

`ui/phone.html` runs the phone app in a browser against a fake engine
(`src/dev/phone-mock.ts`), and `scripts/shoot-phone.sh` photographs it at
390×844, an iPhone's size:

```
npx vite --port 5175 &
scripts/shoot-phone.sh /tmp/shots connections "tab=connections" \
  form "tab=connections&none&press=Add%20a%20connection|A%20shared%20folder"
```

`press=` taps buttons by their words, so screens deeper in a flow can be
photographed. Headless Chrome refuses windows under 500px wide, which is why
the page holds the app to 390 and the script crops.

## 8. Building and running on a phone

From `crates/tungstate-gui/ui`:

```
export JAVA_HOME=/Library/Java/JavaVirtualMachines/jdk-19.jdk/Contents/Home
export NDK_HOME=~/Library/Android/sdk/ndk/29.0.14206865 ANDROID_HOME=~/Library/Android/sdk
TAURI_APP_PATH=.. npm run tauri -- android build --debug --target aarch64 --apk
TAURI_APP_PATH=.. npm run tauri -- ios build --debug --target aarch64-sim
```

The npm package lives in `ui/`, beside the Tauri app rather than above it.
So the generated Xcode and Gradle projects in `gen/` were each edited once
to run their build step from `ui/`.

The end-to-end test used a Samba server in Docker (CI's own image), reached
from the Android emulator as `10.0.2.2`. The emulator can't reach the real
NAS from this Mac: macOS's Local Network privacy setting blocks it.
