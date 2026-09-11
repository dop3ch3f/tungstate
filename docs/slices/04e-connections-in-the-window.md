# Slice 4e: Connections in the window

**Goal:** everything the command line can do with a connection should be
reachable from the desktop app — add one, check it, browse it, drain into it —
because the point of the window is not having to know the command line.

**Runnable outcome:** launch the app → **Connections** → add the NAS over FTP →
**Test** and see what is actually in the root → pick the NAS from a pane's
"Go to…" → browse it → tick files → **Move** → watch **Transfers**. Kill the
app mid-run, reopen, and take Resume or Clean up. All of it without Finder
having mounted anything.

**This brief is the spec.** The tour is in [04e-tour.md](04e-tour.md).

---

## What was actually wrong

Everything remote already worked. A connection, an FTP drain, resume, the
queue, two kinds of stop — all of it shipped in 4b, 4c, 4f and 4g, and all of
it only from the CLI.

The window still believed the world was local. Three lines said so:

- `places()` was five user directories plus a macOS scan of `/Volumes`.
- `browse` opened `Endpoint::local(...)`.
- `App.vue:127` decided "this is the NAS" by testing whether a path string
  started with `/Volumes`.

Together those mean the NAS is reachable from the window *only when Finder has
already mounted it* — and the FTP route, the one that exists precisely for when
the mount does not work, had no way in.

The seams had been cut in advance and the code said so. `backend_for` and
`spawn_run` were already connection-aware, so a CLI-created remote link ran
from the window today. `plan_transfer`'s nesting guard was already written as
`if source.connection == destination.connection`. `browse` already went through
the backend factory rather than `LocalBackend::new`. Four `// slice 4d widens
this` markers pointed at the exact lines. This slice is the widening.

## Decisions

**A pane's location is a single composite string.** `/Users/me/Videos` or
`nas:inbox/2026`, read by the same `parse_end` the command line uses. Not a
`{connection, path}` pair on the wire: the pane's address bar, the "Go to…"
list, the saved pane state, the transfer legs and the link specs are all one
string today, and splitting them would mean changing five shapes to express one
thing. The two-character rule (`C:\Users\me` is a drive letter, `nas:` is a
connection) is what makes one string unambiguous.

**The endpoint helpers move into `tungstate-journal`.** `describe` and `place`
were byte-identical in `cli/src/ends.rs` and `gui/src/main.rs`, and this slice
would have added a third copy. They move with `connection_prefix`, `parse_end`
and `EndError`, because the journal owns `Endpoint` and `Connection` and
`parse_end` needs a `Journal` to resolve a name anyway. Nothing new joins the
journal's dependency set.

Two helpers join them, because the window needs what the CLI never did —
walking *into* and *out of* a remote:

```rust
pub fn join_display(location: &str, child: &str) -> String;
pub fn parent_display(location: &str) -> Option<String>;
```

`Path::join` yields `nas:inbox\a.mp4` on Windows, and `Path::parent` on
`nas:inbox` answers `Some("nas:")` where the honest answer at a connection's
root is `None`. Same rule `Location::display_path` and `remote_key` already
encode.

**The journal gains `update_connection` and `links_using`. No rename.** The
name is the key, not a field: it is what people typed into their link specs and
it derives the keychain account, so a rename is two migrations wearing one hat.
`ConnectionSettings` cannot express one, which is the point — the API cannot
desynchronise the keychain from the journal.

`update_connection` is a full replace of the mutable columns rather than a
partial patch. A form submits every field, and partial-update semantics over
`Option<Option<T>>` is a well-known way to make "clear this field" unreachable.
The CLI layers its `--flag`-shaped partial UX on top.

`links_using` exists so a refused delete can say *which* links block it.
`delete_connection` leans on the foreign key, which knows that something
references the row but not what.

**No password-presence query.** Reading a keychain item from an unsigned binary
prompts on macOS, so probing "does this have a password?" would put a system
dialog on screen just to decide how to draw a button. The UI offers *Change
password* unconditionally instead.

**Editing a connection a link points at is allowed; deleting it is not.** A
link holds a connection *id*, so moving the connection re-points every link at
once — which is the whole reason for the indirection. Deleting would leave
those links pointing at nothing, which is why the foreign key refuses it.

## What ships

1. **`tungstate-journal::ends`** — `connection_prefix`, `parse_end`, `describe`,
   `place`, `EndError`, plus `join_display` and `parent_display`. The CLI's
   `ends.rs` is deleted; `use tungstate_journal::ends` keeps every call site
   reading the same.
2. **Journal** — `ConnectionSettings`, `update_connection`, `links_using`. No
   migration; v5 is unchanged.
3. **CLI** — `connection update <NAME> [--scheme|--host|--port|--user|--root|--option]`
   and `connection password <NAME> [--secret-stdin]`. The second is the missing
   recovery path: `BackendError::Auth` told you to re-enter the password and
   there was no way to, because `connection remove` is refused while any link
   points at the connection.
4. **GUI, six commands** — `list_connections`, `add_connection`,
   `update_connection`, `set_connection_password`, `test_connection`,
   `remove_connection`. `ConnectionView` carries derived `encrypted` and
   `networked` flags so the unencrypted warning is computed once in Rust.
   `test_connection` calls the existing `probe` — a listing, never a write —
   and returns the entry count *and* the root it listed.
5. **GUI, connection-aware browsing** — `browse` parses with `parse_end` and
   builds children with `join_display`; `places()` appends each connection as
   `name:`, which also gives Windows and Linux a "Go to…" list that is no
   longer empty of anything but home; `plan_transfer` and `create_link` route
   through `parse_end` and gain the two refusals the CLI already makes.
6. **GUI, the Connections tab** — `ConnectionsView.vue` and
   `ConnectionModal.vue`, plus three stylesheet gaps closed (`input[type=
   "password"]` had no rule, and `.go` was defined only under `.welcome` and
   `.stranded`, so `TransfersView`'s bare `class="go"` had been picking up
   nothing).
7. **CI** — the flaky `ftp` job fixed at its root.

## The window cannot create a link the CLI would reject

Two refusals the CLI makes at `link add` are now made here too:

- `--move --trash` with a remote source. This desktop's trash knows nothing
  about a NAS, and remote trash is a later slice.
- `replace` against a destination that cannot rename. Replace keeps the file
  already there by moving it aside first, and FTP gives us no rename.

They live in one `refuse_impossible` called from both `plan_transfer` and
`create_link`. Two surfaces of one product disagreeing about the same link is
a rule nobody can rely on.

## The FTP CI job

The container logged `adduser: /ftp/tungstate: No such file or directory` — the
image creates the account before the home directory, so the account existed
pointing at a directory that did not. The workflow tried to repair that in a
step *after* a readiness poll that only checked login, and a login can succeed
in the window before the root is listable. That window was the flake: green on
`slice-4d`, red on `slice-4f`, green on `slice-04-concurrency`, red on `main`,
with no code change to explain any of it.

Two changes, both at the root rather than at the symptom:

- **The home exists before the container does.** A host directory bind-mounted
  at `/ftp/tungstate`, owned by uid 10000 because that is what the `USERS` spec
  pins. The repair step and its retry loop are deleted rather than tuned.
- **Readiness means usable, not merely authenticated.** The poll logs in *and*
  LISTs the root over a passive data connection — which is what the suite's
  first call does, and which also proves the published port range works.

The push trigger gains `fix-**`, because slice 4g shipped on such a branch and
so never got a three-platform run at all.

## Acceptance criteria

- `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`,
  `cargo test --locked` clean; `npm run build` (which runs `vue-tsc --noEmit`)
  clean.
- A NAS added in the window, tested, browsed and drained into, with the app
  killed mid-run and resumed.
- A wrong password reported as credentials refused rather than as a network
  problem, and correctable from the window.
- A connection's root edited while a link points at it, and the link still
  resolves.
- CI green on Linux, macOS and Windows, and the `ftp` job green twice running.

## Tests

**Journal** — `update_connection` round-trips every mutable field and can clear
one that was set; updating a connection a link points at is allowed where
deleting it is refused; `links_using` names the links, both ends counted. The
moved `describe` / `place` / `parse_end` tests carry over unchanged.
`join_display` and `parent_display` get their own: `nas:` has no parent,
`nas:inbox/2026`'s parent is `nas:inbox`, and a child of `nas:inbox` is
`nas:inbox/a.mp4` with a forward slash on every platform.

**CLI** — `connection update` changes only the flags given and leaves the rest;
it refuses an unknown name and an unknown scheme; a connection a link uses can
still be updated; `connection password` reports what it did and refuses a
scheme that never sends one. All through the existing `sandboxed()` harness.

**GUI, the transfer rules** — pure functions, matching the crate's convention.
A cross-connection pair (`nas:inbox` → `/Users/me/inbox`) that path containment
would wrongly refuse is accepted; a same-connection nested pair
(`nas:inbox` → `nas:inbox/2026`) is still refused; `--move --trash` with a
remote source is refused; `replace` is refused against a no-rename destination
and accepted against one that can rename.

**GUI, browsing a connection** — `browse`'s body is split out as `listing_for`,
which takes a `&Journal` instead of a `State`, and is exercised against a real
`fs` connection over a temp directory: a connection root lists its children as
`nas:…` paths and reports no parent; walking two levels down by feeding each
child's own path back in, and then back up one Up-click at a time, round-trips
to the root and stops there; a local pane still lists native paths; a
mistyped connection name is named in the error.

This is a deliberate departure from "pure functions only". `browse` is the
function this slice's whole runnable outcome rests on, and the helpers being
correct in isolation says nothing about them being wired together correctly.
`fs` is the scheme that needs no password, so the test walks a connection
without going anywhere near a keychain.

**Stated gap.** The other five commands — add, update, set password, test,
remove — are untested at the Rust level. They need a keychain or a network, and
no memory secret store was added to the GUI crate. They are thin wrappers over
journal methods that are tested, and the keychain path is proved by hand and
end to end by the CLI suite.

**A second stated gap.** `connection password` replacing a stored secret is not
observable through `sandboxed()`: `TUNGSTATE_SECRETS=memory` is per-process, so
what one invocation stores is gone before the next one starts. The replacement
is proved by an in-process unit test over `store_password`, which is the seam
that does the work; the end-to-end tests cover the reporting and the refusals.

## Out of scope

- **The `quarantined` command.** It walks `std::fs` directly, bypassing the
  backend, so for a remote destination it returns an empty list rather than the
  quarantined files. Registered but unused by the UI. Left alone deliberately;
  it wants its own small fix.
- Connection rename. SFTP, WebDAV, S3. Drag-and-drop onto a connection.
  Byte-offset resume. Remote trash. The `folder` subcommand is still a stub.
- **DMG.** Not a CI matter: `release.yml` exists and its last run was green.
  The failure is a local `tauri build`, and `tauri.conf.json` carries no
  signing identity — the usual cause.
