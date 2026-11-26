# Phone slice E tour: window tabs, and FTP that opens in a folder

The user tried the phone app on their S24 and raised three things:

1. Copying between two places was awkward. ES File Explorer gives each place
   its own window tab; you copy in one and paste in another. Tungstate had a
   second window, but it was hidden under ⋯ › "Open a second window", allowed
   only two, and switched from a bar at the bottom. Nobody found it.
2. An FTP connection saved with "Folder (optional)" left blank failed on the
   first file sent.
3. "Check" on the Connections tab answered with a wall of library text.

The user chose tabs along the top (from three sketches), each storage and
connection opening in its own tab, and for FTP a folder chosen after sign-in
and saved with the connection, so one server can have one connection per
folder.

What changed, by layer:

1. **Engine** (`crates/tungstate-gui/src/lib.rs`): one new command,
   `settings_folders`. `test_settings` now shares its setup with it.
   `update_connection` now forgets the open preview backend, as the password
   and remove commands already did.
2. **Screens** (`ui/src/phone`): window tabs (`state/useWindows.ts`,
   `screens/FilesScreen.vue`), Copy/Move to any open tab (`files/FileWindow.vue`),
   the folder picker (`ui/PFolderPicker.vue`) in the add form and on the
   Connections tab, and plain error sentences (`lib/errors.ts`,
   `ui/PTrouble.vue`).

---

## 1. Why a blank FTP folder failed

Reproduced against a local vsftpd that does not lock the login into its home
folder (Area51 behaves the same way: its `/` is the whole server):

```
$ tungstate connection test ftpt          # root left blank
  / REFUSES files, so every transfer here would fail
$ tungstate connection test ftph          # root /home/tester
  /home/tester accepts files
```

A blank root is the server's `/` (OpenDAL normalises `""` to `/`), and
OpenDAL's FTP writer puts its temporary file at the root whatever the
destination, so even a send into a writable folder fails if `/` refuses
files. The engine already knew this (`Scheme::rootless_warning`, and the
`probe_writable` docs); the phone form just called the folder optional.

The fix is on the phone, not in the engine: FTP connections made on the phone
always carry a folder. What a blank root means for the desktop and the
command line is unchanged.

## 2. `settings_folders`: listing before saving

```rust
#[tauri::command(async)]
fn settings_folders(form: ConnectionForm, secret: Option<String>, path: String)
    -> Result<Vec<String>, String> {
    let connection = unsaved(&ConnectionForm { root: path, ..form })?;
    let listed = with_secret(&connection, secret, |store| {
        tungstate_backend_opendal::probe(&connection, store).map_err(describe)
    })?;
    Ok(folder_names(&listed))
}
```

- `ConnectionForm { root: path, ..form }` is **struct update syntax**: a new
  value with `root` replaced and every other field *moved* out of `form`.
  `form` was passed by value, so the function owns it and can take it apart.
  No clone is needed.
- The trick is the root: each folder the picker opens becomes a temporary
  connection rooted there, and `probe` (the same listing `connection test`
  runs) lists its top. No new backend code.
- `with_secret` takes a closure, `impl FnOnce(&dyn SecretStore) -> Result<T, String>`.
  `FnOnce` is the loosest closure bound: called at most once, so the closure
  may move things it captured. `&dyn SecretStore` lets one closure take either
  store, the in-memory one holding a typed password or the keychain.
  `test_settings` uses the same helper, so the two cannot drift apart.
- `folder_names` sorts with `sort_by_cached_key(|n| n.to_lowercase())`: each
  name is lowercased once instead of on every comparison.

Tested on the `fs` scheme, which goes through the same factory without a
server: folders only, name order, and an error for a path that isn't there.

## 3. Window tabs

`useWindows.ts` keeps its module-level list of windows; the limit went from 2
to 5 and two functions were added:

- `openTab(place)`: for a storage or connection, switch to the tab already
  inside it (where it was left), else use this tab if it is on Files home (a
  blank tab, like a browser's new tab page), else open a new tab, else (all
  five open) use this one.
- `newTab()`: what `+` does.

`within()` decides "already inside": a connection by its name, a volume by
path prefix with a `/` after it, so `/storage/emulated/01` is not inside
`/storage/emulated/0`. `second()` stays as a one-line alias, because Free up
and the screenshot harness call it.

`rehome(top)` sends every tab inside a connection back to its top, which the
Connections tab calls after a folder change. A tab left on `FTP-tester:Photos`
would otherwise point at a `Photos` inside the new folder.

`close()` also had an off-by-one: closing a tab before the current one left
`active` pointing at the next tab. It now steps back by one.
`scripts/phone-windows.test.mjs` covers this and each `openTab` case.

The strip (`FilesScreen.vue`) is always shown, ticking included. The first
build hid it while ticking, and on the emulator that moved every row up 54 px
under the finger, so one long-press selected two files. ✕
sits on the current tab, `+` turns grey at five tabs, and a tab past the edge
scrolls into view. The Copy/Move sheet lists every other tab open on a folder
("Copy to Area51 › Phone"); with none it goes straight to the clipboard, as
before.

## 4. Choosing the FTP folder

In the add form, FTP's main button is **Sign in**, like SMB's share step. It
lists the server's top. The picker opens folders on tap, has ‹ Up, and the
button says **Use Photos**. Then the usual check-and-save runs with that
folder. If the folder refuses files the form says so and stays open. The name
becomes `Area51-Photos` (`nameForFolder`, unique and fitted to the name rule)
unless the person typed their own.

On the Connections tab, ⋯ on an FTP connection offers **Choose folder** (or
**Change folder**) and **Add another folder**. Choose folder uses the saved
password: `test_settings` with no typed secret reads the keychain under the
connection's own name. Add another folder pre-fills the address and login.
The password is typed again, because each connection keeps its own in the
keystore.

A Check on a folderless FTP connection says it needs a folder and shows a
**Choose a folder** button, not the `/` error.

## 5. Plain errors

`lib/errors.ts` maps an error's text to one sentence by pattern (refuses
files, wrong login, no answer, refused, not found, missing folder). Short,
readable text passes through unchanged; anything else becomes "Something went
wrong talking to Area51." `PTrouble.vue` shows the sentence and keeps the
original behind **Details**. The engine's own strings are unchanged, since
the desktop and the command line show them.

## 6. The open backend after a settings change

`Previews` keeps one open backend per connection name (an SMB sign-in is
several round trips). `set_connection_password` and `remove_connection`
cleared it; `update_connection` did not, so after a folder change, pictures
and "open" read from the old folder until the app restarted. This was older
than this slice and on the desktop too. It now calls `state.previews.forget(&name)`
as well.

## Verified

- `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`;
  `npm test` (tabs, names, errors), `vue-tsc`, `check-css`.
- Harness shots: 1, 2 and 5 tabs, the Copy/Move sheet, the FTP form, Choose
  folder, the Check message.
- Android emulator, release build, against a local ProFTPD with no
  `DefaultRoot` (`~/.tungstate-demo/ftp`, `/` lists and refuses files) and
  Docker Samba. Hand-run: FTP sign-in, the folder list, "Use the server's top"
  refused in plain words, `/home/tester` saved as `FTP-tester` and opened in
  its own tab, `+` and Internal storage in a second tab, two files copied
  Download → FTP Photos (sizes match on the server), two copied back, Check
  says reachable, Change folder with the saved password.
- An Alpine vsftpd used first segfaulted (exit 139) part-way. That was the
  test server, not the client; the ProFTPD run had no such trouble.
- Not run here: Area51 itself (it needs the user's login), and the
  `tests::following::*` watcher tests in tungstate-sync, which time out on this
  Mac whatever the code (FSEvents); CI runs them.
