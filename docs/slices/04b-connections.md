# Slice 4b: The connection seam

**Goal:** stop a link end being a bare path. Make it *a place, plus a path
inside it*, so everything downstream — FTP, SFTP, WebDAV, S3, ingest links,
governing a remote in place — plugs into one seam instead of five.

**Runnable outcome:**

```
tungstate connection add scratch --scheme fs --root /tmp/tungstate-dest
tungstate connection test scratch
tungstate link add ~/some-files scratch:inbox --name viaopendal --move
tungstate link preview viaopendal
tungstate link run viaopendal
```

Files land, hashes match, sources are gone — through OpenDAL rather than through
`LocalBackend`, on the same machine, with the same journal. Kill it mid-file and
run it again: nothing lost, nothing duplicated.

**This brief is the spec.** The walkthrough of the shipped code is in
[04b-tour.md](04b-tour.md).

---

## Why the seam ships before FTP

The NAS is reachable two ways: a Finder mount (slice 3, done) and FTP (not
started). Everything after that needs the same widening.

The engine was already ready. `tungstate-transfer` takes `&dyn Backend` for both
ends and never constructs one; the local assumption lived entirely in five
`LocalBackend::new(...)` call sites in the CLI and GUI, and in two `PathBuf`
fields on `Link`. So this is a seam-widening job, not a rewrite.

Doing the seam first has one concrete payoff: it is the part that touches every
crate and needs a schema migration, and it can be proven on all three CI
platforms with **no network at all**, by running the existing transfer suite
through OpenDAL's own `fs` service. Once that is green, FTP is a service
registration plus its quirks.

FTP is slice 4c. The desktop app's connection screens are slice 4d.

## Decisions

**A link end is a connection plus a path.** `Endpoint { connection:
Option<ConnectionId>, path: PathBuf }`. `None` is the local filesystem, and the
path is absolute; `Some` means the path is relative to that connection's root.

**`NULL` in the database, not a `Local` variant.** Migration v4 is additive:
four nullable columns and one new table. Every link and op row written before
this slice means exactly what it meant before, with no backfill and no rewrite.
A `Local` enum variant would have needed every existing row updated, and an
update is a migration that can half-finish.

**No secret ever enters the journal.** There is no `password` column and no
`secret_key` column, so a password cannot leak into a database dump, a backup,
or a bug report. Passwords live in the machine's keychain keyed by connection
name. The absence is structural, not a convention anyone has to remember.

**`migrate` now refuses a newer file, reversing earlier policy.** Until v4 it
silently skipped when `user_version` exceeded what it knew, so an old binary
could still read a newer journal. That was safe while every stored link end was
a local absolute path. It is not any more: a v4 end can be a path relative to a
connection an old binary knows nothing about, and reading it as a local path
would drain into the wrong place. `JournalError::TooNew` is the honest answer.
**This is the decision most worth arguing about in review.**

**SFTP will be hand-rolled on `russh`, not OpenDAL.** Recorded now, implemented
much later. OpenDAL's SFTP service is Unix-only, refuses password
authentication, and delegates host-key checking to the system `ssh` client.
That contradicts three things the design asks for: three-platform support,
trust-on-first-use host keys from `~/.ssh/known_hosts`, and password auth for a
consumer NAS. DESIGN.md §6 already sanctions the escape hatch — "hand-roll a
backend only when OpenDAL's semantics prove wrong for it" — and this is that
case. No SFTP code ships here.

**The adapter owns a tokio runtime and never calls `block_on`.** OpenDAL has
been async-only since 0.54. Its `opendal::blocking::Operator` calls
`Handle::block_on`, which *panics* when the calling thread is already inside a
tokio runtime — and the GUI's Tauri commands are exactly such a thread. So the
adapter spawns each future onto its own runtime and waits on a `std::sync::mpsc`
channel. `spawn` is legal from any thread; `block_on` is not.

**Services registered here: `fs` and `memory` only.** `fs` is not a curiosity.
It is how the adapter gets exercised on macOS, Windows and Linux with no server,
and it answers "does a backend reached through OpenDAL behave identically to
`LocalBackend`?" directly rather than by assertion.

**`name:path` needs a rule, because `C:\Users` is a real path.** A connection
reference is `<name>:<path>` where `<name>` is two or more characters of
`[A-Za-z0-9_-]`. No drive letter is ever two characters, so the two cases cannot
collide. `--from-connection` / `--to-connection` are the unambiguous form and
always win.

**`--move --trash` is refused when the source is remote.** `trash::delete`
drives this desktop's trash and knows nothing about a remote. Remote trash
(`.tungstate-trash/`, DESIGN.md §4) is a later slice. The CLI refuses at
`link add`, and the engine refuses again at transfer time so a link built any
other way cannot delete the wrong local path.

## Shape

Two new crates, one migration, and a factory.

- **`tungstate-journal` v4.** A `connections` table, plus nullable
  `source_connection` / `dest_connection` on `links` and `src_connection` /
  `dst_connection` on `ops`. New module `connections.rs` with `ConnectionId`,
  `Connection`, `NewConnection`, `Scheme` (built with the existing
  `string_enum!` macro so a stored spelling is always a spelling the user could
  have typed), `Endpoint`, and CRUD on `Journal`. `Location` gains the same
  `connection` field. Two queries are **fixed, not extended**: `history` and
  `whereis` reconstructed a full path with `src_root || '/' || src_path`, which
  is wrong for a remote root and was already questionable on Windows; both
  concatenation branches are now scoped to `connection IS NULL`.

- **`tungstate-secret` (new).** `SecretStore` with three methods, plus
  `KeyringStore` (keyring 4.2, service `"tungstate"`, account
  `connection/{name}`) and `MemoryStore`. A crate rather than a module because
  the CLI must store a secret at `connection add` time without dragging in
  OpenDAL, and because tests must never touch a real keychain — headless Linux
  CI has no D-Bus session and therefore no Secret Service.

- **`tungstate-backend-opendal` (new).** Named as DESIGN.md §7 specifies.
  `OpendalBackend` plus `open(endpoint, journal, secrets) -> Box<dyn Backend>`,
  which replaces all five `LocalBackend::new(...)` sites and turns them from
  infallible to fallible. Remote paths go through `remote_key`, a `Components`
  walk reusing the exact escape rule from `local.rs` and joining with a literal
  `/`, because `Path::join` produces `a\b` on Windows and a key with a backslash
  in it is a corrupt key.

- **`tungstate-backend`.** Two additive error variants, `Auth { endpoint }` and
  `Remote { endpoint, operation, source }`.

- **`tungstate-cli`.** `connection add|list|test|remove`; `link add` gains
  `--from-connection` / `--to-connection` and the `name:path` convenience;
  `TUNGSTATE_JOURNAL` and `TUNGSTATE_SECRETS=memory` test overrides so the
  integration tests get their own state and stay parallel-safe.

- **`tungstate-gui`.** No new screens. It constructs `Endpoint::local(...)`
  everywhere, goes through the factory, and the two nesting guards are scoped to
  "both ends in the same place", since path containment says nothing across two
  different connections.

### Two things the build found that the plan had not

**The connection is the rail, not the link end.** The first hand-run of the
sequence above failed: `scratch:inbox` refused to open because `inbox` did not
exist yet. Rooting the operator at the link end made `root_token()` mean "that
particular subfolder exists", which is too strict — a destination folder nobody
has created yet is an ordinary first write. So the operator is rooted at the
*connection* and the link end is carried as a key prefix. The safety rail moves
up one level, to exactly where it belongs: the connection must be there, a
folder inside it need not be.

**`Operator::stat("/")` does not touch the store.** OpenDAL answers it from thin
air with a synthetic directory (`layers/simulate.rs`). Using it for
`root_token()` would have left tungstate's most important safety rail — notice a
NAS unmounting before writing the next file to the boot disk — silently
answering "yes, fine" forever, while the drain deleted originals. The adapter
carries an `Anchor` instead: a local directory gets one `stat` syscall, exactly
as `LocalBackend` does it; anything else lists the root, which is honest but
costs more than it should and is flagged for slice 4c.

## Tests

- A genuine v3 → v4 migration: build a v3-shaped database with raw SQL and
  `PRAGMA user_version = 3`, insert a link and an op, open it, and assert both
  survive with `NULL` connections. The existing test opened twice with current
  code, so it only ever proved idempotence; it is renamed to say so.
- Opening a `user_version = 99` database returns `TooNew`.
- Connection CRUD; duplicate name rejected; deleting a connection a link
  references refused by the foreign key.
- `history` and `whereis` still find local rows and do not match a remote row on
  a concatenated path.
- `MemoryStore` round-trip; `KeyringStore` round-trip `#[ignore]`d.
- **The whole transfer suite, run again through the adapter.** `Rig::run_over`
  already existed for swapping in a fake destination; it now also points at an
  `OpendalBackend` on `services-fs`, including the interrupt-and-resume test,
  the verification-failure test the whole project exists for, and the
  destination-vanishes test.
- Proptest: no generated `Path` yields a key containing `\`, a `..` component,
  or a leading `/`.
- CLI integration tests for `connection add/list/test/remove`, and for end
  parsing: `nas:inbox` resolves, `C:\Users\x` stays local, `unknown:path`
  errors, `--move --trash` on a remote source errors.

Every test uses its own temp directory and temp journal. Nothing touches the
real keychain or the real data directory.

## Acceptance criteria

- [x] `cargo build --locked` clean, `cargo test --locked` green in both parallel
      and single-threaded runs
- [x] `cargo clippy --all-targets --locked -- -D warnings` silent
- [x] `cargo fmt --all --check` silent
- [x] The runnable outcome above, performed by hand
- [x] Killed mid-file with `kill -9` on a 400 MB transfer and re-run: partial
      removed, operation re-queued, hash matches, no duplicates
- [ ] CI green on Linux, macOS and Windows

## Out of scope, deliberately

FTP (4c). GUI connection screens (4d). SFTP, WebDAV, S3. Byte-offset resume.
Remote trash. A `native` verify level using a server-side checksum — the branch
where `Size` and `Hash` diverge stays as it is until a backend can actually
offer one. Concurrency, bandwidth caps and reachability pausing are still slice
4 and still unbuilt.
