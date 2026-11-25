# Phone slice D tour: a file manager you can use

The user tried the phone app and called it unusable. Part of that was that
their phone still ran the Oct 31 build (slice C never reached it). The rest
was real: no previews, one list/grid toggle, a tap on a NAS file only ticked
it, no way to find the NAS, two tabs that only said "coming later", and the
Android back button closing the app. ES File Explorer is the yardstick.
The screens follow the mockups approved on 2026-11-25 (kept on the Mac, not in
the repo: their sample photos are Apple wallpapers).

What changed, by layer:

1. **Engine** (`crates/tungstate-gui/src`): `discover.rs` (servers on the
   network), `network.rs` (the two commands the form calls), `previews.rs`
   (pictures of files, copies of NAS files to open), `browse` can list hidden
   files.
2. **Backends**: `tungstate-backend-smb::list_shares`, and
   `tungstate-backend-opendal::smb_shares` in front of it.
3. **Plugin** (`crates/tauri-plugin-photos`): thumbnails from Android's own
   decoders, the status and navigation bar heights, a multicast lock.
4. **Screens** (`ui/src/phone`): three views with previews, the viewer, back
   button, discovery, the new add-connection form, Duplicates and Organise.

---

## 1. Discovery: two ways of looking, merged by address

`discover::look` runs two lookers on their own threads and merges what they
send down one channel:

```rust
let (tx, rx) = mpsc::channel::<Hit>();
let announced = { let tx = tx.clone(); std::thread::spawn(move || listen(&tx)) };
let knocked = std::thread::spawn(move || knock(&neighbours(&own_addresses()), &tx));
for hit in rx { if let Some(row) = merge(&mut known, hit) { seen(&row); } }
```

`mpsc` is "many producers, one consumer". Each looker owns a `Sender`
(`tx.clone()` makes another), and the `for hit in rx` loop ends by itself
when every sender has been dropped, which happens when both threads finish.
No flag or count says "done"; the channel closing is the signal.

- **mDNS** (`mdns-sd`, built with no async runtime) browses `_smb._tcp`,
  `_ftp._tcp`, `_sftp-ssh._tcp`, `_ssh._tcp`, `_webdav._tcp`,
  `_webdavs._tcp` for four seconds. Area51 announces SMB and FTP.
- **The knock** is a bare TCP connect to 445, 21 and 22 on every address in
  the /24 around this device, 64 at a time, 400 ms each. Nothing is sent.
  HTTP ports are left out on purpose: every router answers on 80.

Two choices a reviewer might question:

- **Only private /24s.** `neighbours` filters with `Ipv4Addr::is_private` and
  takes the /24 around each address whatever the netmask says, so a phone on
  a carrier or a /16 office network never knocks on thousands of hosts.
- **An announced port beats a knocked one.** A server may announce FTP on
  2121; the knock only tries 21. `merge` lets a named sighting set the port.

`merge` returns `Option<Found>`: `Some` only when the row changed. The
command emits `discover://found` for each, so the list fills in during the
look. `(*row != before).then(|| row.clone())` is the idiom: `bool::then`
turns a condition into an `Option`.

**One look at a time.** `network::discover` holds a `static LOOKING:
Mutex<()>`. `try_lock().ok()?` returns `None` straight away if a look is
running, and the window keeps the events it is already getting. The guard
is held in `_looking` until the function returns. A leading underscore
keeps a variable alive without a warning; a bare `_` would drop it at once.

**Android drops multicast** addressed to other apps unless the app holds a
`WifiManager.MulticastLock`, so the command takes one for the look (Kotlin
`multicast`). The manifest gains `CHANGE_WIFI_MULTICAST_STATE`,
`ACCESS_WIFI_STATE` and `ACCESS_NETWORK_STATE`. Target SDK stays 36; at 37
`ACCESS_LOCAL_NETWORK` becomes a runtime prompt and needs asking for.

## 2. SMB shares: picked, not typed

`list_shares` signs in to `IPC$`, the server's pipe share, opens the
`srvsvc` pipe and makes one DCE/RPC call, `NetrShareEnum` at level 1. It
keeps disk shares that are not "special" (`C$`, `ADMIN$`) and drops anything
ending in `$`.

**Why it is written by hand** (`shares.rs`). smb-rs has a `list_shares`,
and the first version used it. Against the Samba test server it failed:
smb-rs binds with the NDR64 transfer syntax only, and Samba answers
`ProposedTransferSyntaxesNotSupported`. Samba is what nearly every home NAS
runs, so that would have failed on the user's own NAS. Classic NDR is the
syntax every server accepts, Windows included, so `shares.rs` encodes the
three things this one call needs: the bind, the request, and the reply's
array of names. smb-rs still carries them, through
`fsctl_with_options(PipeTransceiveRequest)`, which writes a request to the
pipe and reads the answer in one round trip.

Reading NDR is mostly alignment. `Reader` keeps an offset into the reply,
and every `u32` first rounds it up to a multiple of four
(`usize::next_multiple_of`). Strings are UTF-16, so a three-letter name
takes 8 bytes and the next field starts after 2 bytes of padding. The test
`an_odd_length_name_keeps_the_next_one_aligned` holds that. Every read goes
through `take`, which checks the length first, so a reply that is cut short
is an `Err`, never a panic: `a_cut_short_reply_is_an_error_not_a_panic`.

Two Samba tests cover the real thing: the share is listed and no `$` share
is, and a wrong password is `BackendError::Auth`. The client set-up that
`connect` had inline is now `client_for(settings)`, shared by both, so the
two can never sign in differently.

## 3. Pictures of files

The window asks `thumb://localhost/f?p=<location>&s=<edge>` (on Android
and Windows, `http://thumb.localhost/...`, Tauri's arrangement; the old code
only handled Windows, which `engine/pictures.ts` fixes). The scheme was
synchronous; it is now `register_asynchronous_uri_scheme_protocol`, and the
work runs on `spawn_blocking`, because a picture from a NAS is a network
read and must not hold up the window's thread.

How each is made (`previews::picture`):

| Where | How |
| --- | --- |
| This phone | The plugin's `thumbnail`: `ThumbnailUtils` for photos (HEIC too) and video frames, the song's cover, an APK's icon |
| Free up's findings | `content://media/...` through `loadThumbnail`. Checked before anything else: `content:` would read as a connection called "content" |
| A connection | The preview a camera stores in the first 66 KB (`tungstate_likeness::preview::from_head`); without one, the whole photo up to 40 MB, shrunk by the phone |
| Desktop | Pictures only, through `tungstate_likeness::look` |

Things worth knowing:

- **One backend per connection** (`Previews::backends`). An SMB sign-in is
  several round trips and a folder asks for dozens of pictures. The map holds
  `Arc<dyn Backend>`: `Arc::from(box)` turns the factory's `Box<dyn Backend>`
  into a shared pointer every request can clone cheaply. It is forgotten
  when the connection's password changes or it is removed.
- **A counting semaphore** (`Gate`), since std has none: a `Mutex<usize>` of
  free places and a `Condvar` to wait on. `enter` returns a `Pass` whose
  `Drop` gives the place back, so a picture that fails part-way can never
  keep its place. That is RAII: the cleanup is tied to a value going out of
  scope rather than to remembering to call it.
- **What may be asked for.** Local paths must be absolute with no `..`; on a
  phone, only under `/storage/` or the app's cache. A remote path must name
  a saved connection. Only kinds that have a picture are tried.
- **The cache** has its own folder (`pictures/`), named by path, size and
  mtime as the dupes thumbnails are, and `Thumbs::trim` keeps it under
  300 MB, oldest first, after every 200 new pictures.

**Opening a NAS file**: `fetch_copy` copies it into the cache's `opened/`
folder, emitting `fetch://progress`, and answers with the path, which the
phone's app for its kind opens through the existing FileProvider
(`cache-path` was already in `file_paths.xml`). `opened/` is cleared when
the app starts.

## 4. The file window

**Three views** (`lib/views.ts`): List, Detailed list, Tiles. The choice is
remembered as before, and a stored `"grid"` from slice C reads as Tiles, so
nobody's setting is lost. A fresh install shows Detailed list.

**Only the rows on screen are drawn** (`lib/virtual.ts`). Every row in a
view is one height, so which rows are visible is arithmetic:
`rowsInView(top, height, rowHeight, rows)` gives `[first, end)` with six
spare rows each side. The list is one tall box with a slice of real rows
moved into place with `translateY`. A camera folder with 8,000 photos draws
about 30 buttons. Slice C's "first 60 rows" cap is gone from the file
window; Free up keeps its own.

**`PThumb`** draws the kind's mark first and lays the picture over it when it
arrives (`loading="lazy"`), so a slow NAS never leaves a hole.

**The viewer** (`files/Viewer.vue`) shows the folder's photos one at a time
on the retro dark desk: swipe for the next, pinch or double-tap to zoom,
Share, Move, Info and Delete. It asks for a screen-sized picture (at most
1600 px), so a HEIC shows and a 50 MB photo is not loaded whole. The next
photo is fetched while you look at this one.

**A tap opens.** A folder opens; a photo opens the viewer; anything else
goes to the phone's app for it, fetched first from a connection with a
progress bar in its row.

Also: the current folder is the last, dark path chip; Show hidden files in
⋯ (the `browse` command takes an optional `hidden`, so an older window that
never sends it is unchanged); pull down to refresh; Info and Open with… for
one ticked file; each folder's scroll position is kept for when you come
back up.

**The back button** (`lib/back.ts`): whatever is on screen registers a step
while it is mounted (`useBack`), and `stepBack` asks the newest first. A
sheet closes, then the viewer, then selection ends, then the folder goes up,
then another tab goes to Files, and at Files home a second press within two
seconds leaves (`core:app:allow-exit`, in a phone-only capability). The
real bar heights come from the plugin's `insets` instead of the guessed
40 px.

## 5. Discovery and the add-connection form

"On your network" sits on Files home and in Connections, scanned when either
opens, with Look again. Servers already saved (by address or `.local` name)
are not offered. Tapping one opens the form with the kind, address, port and
name filled in (`lib/discovery.ts`: SMB before SFTP before WebDAV before
FTP; the name fitted to the name rule, never one already taken).

For SMB the first button is **Sign in**, which lists the shares; pick one,
then **Save and open Photos**. If a server refuses to list its shares, the
share is typed instead. FTP and SMB have **Sign in as guest**; FTP warns that
it sends the password unencrypted, naming the safer kind the same server
offers. Password fields have Show. Check and Save are one press: it checks,
then saves.

## 6. Duplicates and Organise, on Android

They said "coming later" only because they had not been built for the phone.
Both use the desktop's engine as it is.

**Duplicates** (`screens/DupesScreen.vue`) reuses the desktop's `useDupes`
store. Choose where (`PPlaces` offers the whole phone, Camera, Pictures,
Download and WhatsApp, only those that exist), find, then groups with
previews: ★ is the copy that stays, the extras are ticked, and the last copy
of a group can't be ticked. Clear sets the extras aside in
`.tungstate-quarantine` inside the place scanned, so Put back works. **Empty
it** deletes that folder for good, through the journaled delete. One place at
a time: the engine scans one root, and the whole phone is the default so
copies across folders (Camera and WhatsApp) are found.

**Organise** (`screens/OrganiseScreen.vue`): choose a folder; if it has no
rules yet, every starting layout is tried on paper (`compare_folder`, nothing
written) and one is suggested by the folder's name (Download → `downloads`).
Choosing one writes it as the folder's rules, then the preview shows where
the files go, grouped by destination folder (`lib/organise.ts`), then
Organise, then Put back.

Both are hidden on iOS, where an app cannot see these folders.

## 7. Found by trying it on Android

Three things only the real build showed:

- **The window froze during an SMB sign-in.** `browse`, `test_settings` and
  `test_connection` were plain `#[tauri::command]`s. A plain command runs on
  the window's own thread, so for the seconds an SMB server took to answer,
  taps queued up and played back afterwards. All three are now
  `#[tauri::command(async)]`, which runs them on Tauri's thread pool. The
  contract with the window is the same, and the desktop gains the same fix.
- **"The whole phone" could not be scanned for duplicates.** The folder walk
  stops at the first folder it cannot read, and `Android/data` and
  `Android/obb` are unreadable to every app. `dupes::probe_for(phone)` adds
  them to the scan's ignore list on Android only. The shared walk is not
  changed, because tidying a desktop folder should still stop rather than
  quietly skip what it cannot see. Test:
  `a_phone_scan_leaves_out_every_apps_private_folders`.
- **Search opened without the keyboard.** The field now takes focus when it
  opens.

## What is not in this slice

- **iPhone** discovery (NWBrowser and the Local Network prompt) and the
  desktop's add-connection form. The engine command is shared, so both are
  window work only.
- **Video previews from a NAS**: they keep their mark.
- **Several folders at once** for Duplicates; the engine scans one root.
- **Emptying the holding folder** leaves the clear-out in History; its Put
  back then has nothing to put back and says so.

## Checks

- `npm run build` (CSS check, vue-tsc, Vite) and `npm test`: 75 pass,
  including `phone-discovery`, `phone-back`, `phone-virtual` and
  `phone-organise`.
- `cargo test --workspace`: everything passes except the four watcher tests
  this Mac's FSEvents is known to break (`tests::following::*` in
  tungstate-sync, one in tungstate-watch), which CI runs. New here:
  `discover` (7), `previews` (6), hidden files, `Thumbs::trim`, the phone
  scan's ignore list, and `shares` (6).
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`. The
  fmt check also fixes the `volumes.rs` line that failed CI on slice C,
  after which CI stopped before clippy and tests ran on Linux and Windows.
- Samba: `cargo test -p tungstate-backend-smb --features samba-integration`:
  15 pass against the Docker server, `list_shares` among them.
- `looks_at_this_network` (by hand, `--ignored --nocapture`) found Area51
  with SMB and FTP in under five seconds.
- On the Android emulator, release build: previews in all three views on
  the phone and on the Docker share, the viewer (swipe, back), the back
  button up through folders, discovery finding Area51, adding the Docker
  server by hand with the share picked from the list, Duplicates on the
  whole phone, Organise with a preview, organise and Put back, and Free up's
  findings with previews.
- Harness shots of every new screen: `scripts/shoot-phone.sh` with
  `?view=list|detailed|tiles`, `?tab=dupes`, `?tab=organise`,
  `?tab=connections&none`.
