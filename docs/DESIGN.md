# Tungstate Design

## Context

Tungstate is a cross-platform (Rust) tool that governs folders. A governed folder has a declared structure. Tungstate watches it, detects disorder (misplaced files, naming violations, duplicates), and reconciles automatically. It must work on local disks and on remote endpoints (FTP/SFTP/S3/WebDAV), because the pain that prompted it is half-baked FTP drop folders that rot into chaos.

No code exists yet. This document is the brainstorm: every subsystem, the options, and a recommendation for each. Decisions marked **OPEN** need your call. Decisions marked **DECIDED** were settled in conversation.

## Working mode (DECIDED)

The user builds tungstate themselves. Claude acts as PM and senior Rust engineer: proposes the next slice, names the concepts and crates to learn, sets acceptance criteria, reviews the code, corrects idioms and design, and explains the why. Claude writes code only when the user explicitly hands over a named slice. Slow by design. This document doubles as the product spec and the syllabus.

## The origin story (DECIDED, drives priorities)

A full MacBook holds videos that must go to a NAS. The move must be file by file: copy one, verify it, delete the source, reclaim the space, next file. It must survive closing the lid and leaving the house, then resume with nothing lost and nothing duplicated. No FTP client, sync tool, or file manager does this well. This "durable drain" is the first thing tungstate must do flawlessly (§4b). Governance of the destination tree (§1–§4) is what happens to files once they arrive.

The one-line pitch that should survive every decision below:

> **A reconciliation controller for filesystems.** You declare the desired shape of a tree. Tungstate continuously computes the diff between desired and actual, and applies it safely. Like Kubernetes controllers or Terraform, but the resources are files.

That framing is what separates it from Hazel / organize / Maid (imperative "if X then move" rules with no notion of desired state, no drift detection, no undo, no remotes) and from rclone / Syncthing (sync, not structure).

---

## 1. Mental model and vocabulary

Getting the nouns right early prevents a muddled API later.

| Term | Meaning |
|---|---|
| **Folder** | **DECIDED noun.** A governed root (local path or remote URL) plus its policy. The unit of daemon management. Convention to avoid ambiguity: in tungstate's docs, CLI, and code, *folder* always means a governed folder; any other directory is a *directory*. `tungstate folder add ~/Downloads`. |
| **Policy** | The declarative file describing the folder's desired structure and behaviour. Lives at `<root>/.tungstate/policy.toml` (or centrally, see §7). |
| **Schema** | The part of the policy that describes the tree shape: which directories exist, what each is allowed to contain. |
| **Classifier** | The part of the policy that maps a file's attributes to its canonical location. |
| **Canonical path** | The one place a given file *should* be, as a pure function `f(attributes, policy)`. |
| **Drift** | Any difference between actual state and canonical state. Misplaced file, duplicate, forbidden file, missing required directory, empty directory. |
| **Plan** | An ordered set of operations that eliminates drift. |
| **Journal** | Append-only log of every operation applied, sufficient to undo. |
| **Inbox** | A directory the policy designates as "unclassified things land here." |
| **Quarantine** | Where tungstate parks things it refuses to decide about (conflicts, unclassifiable, suspicious). |

Core invariant, worth writing on the wall: **reconciliation is idempotent and convergent.** Running it twice yields no ops the second time. Running it on an already-conformant tree yields nothing. This is the property that makes "run it continuously" safe, and it's the property to test with property-based tests.

Second invariant: **no content is ever lost.** The multiset of content hashes in (folder ∪ trash ∪ quarantine) after a plan equals the multiset before. Dedup reduces copies, never removes the last one.

### Declarative schema vs imperative rules — **DECIDED: hybrid, two skins on one model**

The user wants both a layout template such as `{Year}/{Month}/{Source}/{Category}/{Type}/{SizeRange}/` and Hazel-style rule lists for less technical users. These are the same thing underneath:

- A **layout** is one `[[dir]]` entry whose `path` is a big template. It routes everything it matches into a computed subtree.
- A **rule** is a `[[dir]]` entry with a narrow `match` and a mostly literal `path`.
- A policy is an ordered list of these. Most-specific match wins, ties are a load-time error.

So "rule list mode" is just a policy with no template variables, and "layout mode" is a policy with one or two rich entries. Same parser, same planner, same `explain`. The two skins are documentation and `init --template` presets, not two engines. This is the key simplification: **do not build two policy systems.**

The `{Source}` variable is worth a note: it means "which ingest link, device, or app produced this file" and is only knowable from the journal (§4c). Provenance feeds placement. That is why the journal is core, not a logging afterthought.

`{SizeRange}` and `{Category}` are **bucket variables**: user-defined mappings from a raw attribute to a label. The template language needs a `bucket` provider (`size → ["<10MiB", "10MiB-1GiB", ">1GiB"]`) and a `map` provider (`mime → category`). Ship sensible default buckets and let policies override.

---

## 2. Policy language

### File format — **OPEN (Q2)**

| Option | Pros | Cons |
|---|---|---|
| **TOML** | Rust-native (serde), ubiquitous, cargo users know it | Nested trees get verbose; no comments-in-arrays nicety |
| **KDL** | Designed for tree-shaped config, reads like a directory listing, `kdl` crate is solid | Less known; fewer editors highlight it |
| **YAML** | Everyone knows it | Footguns (Norway problem, indentation), weakest choice |
| **Embedded scripting (Rhai/Lua)** | Unlimited expressiveness | You now maintain a runtime; policies aren't statically analysable; not "declarative" |

Recommendation: **TOML for v1**, with a tiny expression/template language inside string values (see below). Keep a scripting escape hatch (Rhai, pure Rust, sandboxable) on the roadmap for custom classifiers, gated behind explicit opt-in. KDL is the strongest alternative and worth a look if TOML nesting gets ugly in prototyping.

### Sketch of a policy

**DECIDED:** entries are `[[rule]]`; the file lives at `<folder>/.tungstate/policy.toml` for local folders (travels with the folder, can be committed) and in the central config directory for remote folders. A central file may also exist for a local folder; the in-folder file wins.

```toml
[folder]
name = "downloads"
mode = "enforce"           # observe | suggest | enforce
inbox = "_inbox"           # unclassified files go here instead of staying put
ignore = [".DS_Store", "Thumbs.db", "*.part", "*.crdownload", ".git/**"]
opaque = ["*.app", "*.photoslibrary", "**/.git", "**/node_modules"]  # treat as single atomic units, never look inside

[defaults]
on_duplicate = "trash"     # skip | trash | replace | newer-wins | larger-wins | hardlink | reflink | quarantine | keep-both
on_conflict  = "quarantine" # same name, different content at target
cooldown     = "30s"        # wait this long after last write before touching a file

# Layout skin: one rule, big template
[[rule]]
name = "media-by-origin"
path = "{date:%Y}/{date:%m}/{source}/{category}/{ext}/{size_range}"
match = { mime = ["image/*", "video/*"] }
vars.date       = { from = ["exif.DateTimeOriginal", "video.created", "mtime"], tz = "local" }
vars.category   = { from = "mime", map = { "image/*" = "Photos", "video/*" = "Videos" } }
vars.size_range = { from = "size", bucket = ["<10MiB", "10MiB-1GiB", ">1GiB"] }
rename = "{date:%Y%m%d_%H%M%S}_{hash:8}.{ext|lower}"

# Rule-list skin: narrow match, literal path
[[rule]]
name = "archives"
path = "Archives"
match = { ext = ["zip", "tar", "gz", "7z"], size = "> 1MiB" }

[[rule]]
name = "documents"
path = "Documents/{kind}"
match = { mime = ["application/pdf", "application/msword", "text/*"] }
vars.kind = { from = "mime", map = { "application/pdf" = "PDF", "text/*" = "Text", "_" = "Other" } }
```

### Rename-template filters (DECIDED set for v1)
`lower`, `upper`, `slug` (ASCII, dashes), `sanitize` (strip characters illegal on the target backend, always applied last), `trunc:N`, `pad:N`, `hash:N` (prefix of BLAKE3 hex), date formatting via `strftime` specifiers, `default:"x"`. Filters chain with `|`. Anything beyond this waits for the Rhai escape hatch rather than growing a bespoke language.

### Attribute vocabulary (what templates can reference)

- **Name-derived:** `name`, `stem`, `ext`, regex captures on name
- **Stat-derived:** `size`, `mtime`, `ctime`, `birthtime` (where available), `is_dir`, `is_symlink`
- **Content-derived (costs a read):** `mime` (via `infer` magic bytes, fall back to extension), `hash` (BLAKE3), `exif.*`, `id3.*`, `pdf.title`, `video.duration`
- **Context-derived:** `parent`, depth, `folder.name`, `now`
- **Remote-aware:** attributes have a cost tier. Stat is free, magic-byte sniff needs the first 8 KiB (range read), EXIF needs the first ~64 KiB, hash needs the whole file. The planner should be able to say "this classification needs tier N" and the backend fetches only that much.

### Matching semantics

- Globs (`globset`), regex, mime, size ranges, age ranges, boolean composition.
- **Precedence:** most-specific match wins; ties broken by explicit `priority`; still-tied is a policy validation error at load time, not a runtime surprise. This is the single most important design choice for avoiding Hazel-style "why did it go there" confusion.
- `tungstate explain <file>` prints the full decision trace. Build this from day one; it's also your debugger.

### Directory strictness

- `strict = true`: only files the classifier routes here may exist here. Anything else is drift.
- `strict = false` (default): classifier routes files in, but existing unmatched files are tolerated.
- Missing directories in the schema: created lazily on first placement (default), or eagerly (`ensure = true`).
- Empty directories left behind by moves: pruned if tungstate created them, left alone if the user did (the journal knows which).

### Templates and built-ins

Ship named templates users can `tungstate init --template photos|downloads|documents|media-library|projects`. They are just policy files in the binary. Custom templates from `~/.config/tungstate/templates/`.

---

## 3. Observation: knowing what's on disk

### Initial scan
- Parallel walk (`jwalk` or `ignore` crate's parallel walker). Respect `ignore` and `opaque` patterns so `node_modules` and `.git` are never descended.
- Emit stat-level entries into the index. Do not hash on scan. Hash lazily when a classifier or the dedup pass asks.
- Large trees (millions of files): scan is streaming, index writes are batched in transactions.

### Watching (local)
- `notify` crate (FSEvents / inotify / ReadDirectoryChangesW) plus `notify-debouncer-full` to coalesce rename pairs and bursts.
- **Partial-write problem:** a browser writes `foo.crdownload` then renames; an SFTP upload grows for minutes. Solutions, layered: ignore patterns for known temp suffixes; size-stability cooldown (no change for N seconds); on Linux/macOS check whether any process holds it open (`lsof`-equivalent is expensive, so only as a final check on large files).
- **Watchers drop events** (inotify queue overflow, FSEvents coalescing). Always run a periodic full rescan (default hourly, configurable) as the source of truth. The watcher is an optimisation, not the truth.
- **Self-event suppression:** every op tungstate performs will echo back as a watcher event. Keep an "expected events" set keyed by path with a short TTL; matching events are swallowed. Without this you get feedback loops.

### Watching (remote)
- No push events for FTP/SFTP/WebDAV. S3 can push via SQS/EventBridge but that's opt-in complexity.
- Poll with listing diffs. Listing cost is the budget: adaptive interval (fast when things are changing, slow when quiet). Compare against index by (path, size, mtime) and only stat/fetch what changed.
- The `Backend` trait exposes `watch() -> Option<EventStream>`. `None` means poll.

### The index — **OPEN (Q6)**
Store per folder: path, size, mtime, platform file id (inode/dev or Windows file index), hash(es) with the (size, mtime) they were computed at, classification result, last action, quarantine reason.

- **SQLite (`rusqlite`, bundled):** queryable, well understood, single file, WAL mode is fine for one writer. Cross-platform with zero system deps when bundled.
- **redb:** pure Rust, embedded, ACID, typed tables. No SQL, so "find all files with hash X" needs a secondary table you maintain by hand.
- Recommendation: **SQLite.** The queries you'll want (`by hash`, `by path prefix`, `drift since`, `history for path`) are relational, and the debugging story (`sqlite3 index.db`) is unbeatable.

Location: `<root>/.tungstate/` for local folders (travels with the folder), `~/.local/share/tungstate/folders/<id>/` for remote folders (no writing metadata into someone's FTP).

---

## 4. Reconciliation engine

The heart of the project. Design this as a pure library with no I/O so it's testable with an in-memory fake backend.

### The loop
```
observe  →  classify  →  diff (desired vs actual)  →  plan  →  execute  →  journal  →  (repeat)
```

### Planner
- Input: index snapshot + policy. Output: a `Plan` = DAG of ops.
- Op types: `MkDir`, `Move`, `Rename`, `Copy` (cross-device), `Link` (hard/sym/reflink), `Trash`, `Delete` (only from trash, only on explicit purge), `Quarantine`, `Noop(reason)`.
- Ordering matters: moving A to B where B is occupied by C moving to D requires D-move first. Cycles (A↔B swap) need a temp name. Build the dependency graph, topo-sort, break cycles with temp renames. This is a classic problem; treat it as one.
- Plans are serialisable (`plan.json`) so `tungstate plan` can be reviewed, saved, and later `tungstate apply plan.json`.

### Safety rails (non-negotiable list)
1. **Never touch anything outside the folder root.** Canonicalise every path and check the prefix. Symlinks pointing out are never followed.
2. **Never delete.** "Delete" means move to trash (`trash` crate uses the OS trash; remotes get `.tungstate-trash/` with a retention policy). Real deletion is a separate explicit `purge` command.
3. **Blast-radius circuit breaker.** If a plan would touch more than N files or N% of the folder (default: 500 or 20%), refuse to auto-apply. Print the plan and require `--yes`. This is what saves you when you typo the policy and it wants to reshuffle 80k photos.
4. **Dry-run everything.** `plan` is the default; `apply` is explicit; daemon mode is explicit opt-in per folder via `mode = "enforce"`.
5. **Journal before act.** Write the intended op to the journal, execute, mark done. Crash recovery replays or rolls back incomplete ops on startup.
6. **Undo.** `tungstate undo [--last N | --since <time> | --plan <id>]` reverses journal entries. Reverse of Move is Move; reverse of Trash is Restore; reverse of Link-dedup is Copy back.
7. **Verify after transfer.** Cross-device or remote moves are copy → verify hash → delete source. Never delete the source on an unverified copy.
8. **Cooldown.** Newly arrived or recently modified files are left alone for a configurable window.
9. **Opaque bundles.** macOS `.app`, `.photoslibrary`, `.git`, `node_modules`, and anything user-listed are atomic. Reorganising the inside of a git repo is a bug, not a feature.

### Modes
- `observe`: scan and report drift; never plan.
- `suggest`: plan and surface it (CLI, notification); apply only on approval.
- `enforce`: apply automatically, subject to the circuit breaker.
Per-folder default with per-directory override, so a folder can enforce `Photos/` but only suggest for `Projects/`.

### Concurrency
- One reconciliation loop per folder, folders in parallel.
- Op execution: bounded parallelism (default 4), never two ops touching the same path family concurrently.
- The user is editing the tree at the same time. Every op re-stats its source immediately before acting and aborts if size/mtime changed since planning. Stale plan → replan, not stale action.

---

## 4b. Durable transfer: drain and ingest (DECIDED: both directions in v1)

The primitive the whole project was born from. One engine, two directions:

- **Drain (push):** any directory A → destination B, removing from A as each file lands. The MacBook-to-NAS case.
- **Ingest (pull):** remote source → local folder inbox, optionally removing from the source. The FTP drop-folder case.

Both are a `Link { from, to, direction, remove_source: bool, policy }`. The transfer engine does not know or care which is "local."

### Per-file state machine
```
Discovered → Queued → Copying(offset) → Verifying → Committed → SourceRemoved
                 ↘ Paused / Retrying(n) / Failed(reason) / Skipped(reason)
```
Every transition is journaled **before** it happens (write-ahead). A crash between any two states is recoverable by reading the last journaled state. The source is deleted only from `Committed`, and `Committed` is only reached after verification.

### Copy step
- Stream to a temp name on the destination (`name.tstpart` beside the target, or a `.tungstate/partial/` area if the backend allows). Never write to the final name until verified.
- Hash the stream while copying (BLAKE3). One read of the source produces both the copy and the expected hash.
- **Resume:** if a partial exists on the destination, resume from its length when the backend supports offset writes (SFTP, FTP `REST`/`APPE`, S3 multipart, local). Otherwise restart. Record the offset in the journal every N MiB so a resumed copy trusts only journaled progress, not just file length.
- Preserve metadata after the data is in: mtime, permissions, xattrs (Finder tags, comments) where the backend supports it. Record per file what could **not** be preserved so the "no details lost" promise is honest.

### Verify step — verification level is a per-link knob
| Level | Cost | Trust |
|---|---|---|
| `size` | free | weak, catches truncation only |
| `native` | free-ish | backend checksum (S3 ETag, SFTP `check-file`, WebDAV `getetag`); only where the backend computes it server-side |
| `readback` | full second read over the network | strong; the only option that proves the bytes on the disk are the bytes you sent |

**DECIDED:** all three levels are offered per link. Default is `hash` (stream hash while sending plus destination size check). `readback` is the opt-in for the paranoid run, `native` when the backend cooperates. The CLI prints the level in effect when a link starts so nobody is surprised later.

### Commit and reclaim — **DECIDED: explicit, no silent default**
- Atomic rename temp → final (or copy+delete where the backend cannot rename; capability flag).
- Source handling is a required choice when the link is created, because users differ (the author is storage-choked, others are not): `tungstate link add A B --move` (delete source after verify, reclaims space now), `--move --trash` (source to OS trash, reclaims nothing until emptied but is reversible), or `--copy` (source untouched, link behaves as a verified mirror). Omitting all three is an error with a one-line explanation, not a guess. In the policy file: `source = "delete" | "trash" | "keep"`.
- Prune empty source directories tungstate emptied, never ones it did not.
- If the destination is a folder, the file enters its inbox and the normal reconcile loop places it. Origin link name becomes the file's `{Source}`.

### Ordering strategies
`largest-first` (reclaims space fastest, default for drain), `smallest-first` (most files committed per minute), `oldest-first`, `directory-order`. Configurable per link.

### Interruption and reachability
- Destination unreachable → link goes `Paused(unreachable)`, no error spam, probe with backoff, auto-resume when reachable. Leaving the house must be a non-event.
- Conditions to run: `only_when = ["destination_reachable", "on_ac_power", "on_network:HomeWifi", "idle"]`. Start with reachability and AC power; Wi-Fi SSID detection is platform-specific and can come later.
- Bandwidth cap and concurrency (default 2 parallel files for a NAS; more just thrashes spinning disks).
- Source file modified during copy (hash mismatch on verify, or size/mtime changed) → back to `Queued` after cooldown, never committed.

### Conflicts at destination
Same name exists: same hash → treat as already transferred, remove source, journal as `Deduplicated`. Different hash → link policy: `rename` (suffix), `skip`, `replace` (existing goes to trash), `newer-wins`, `larger-wins`, `quarantine`.

### Why existing tools fall short (for the README, and to keep us honest)
`rsync --remove-source-files` is the closest. It lacks per-file journaled state, readback verification, resume-from-offset semantics you can trust, reachability-aware pausing, and any answer to "which files are still on the laptop." Finder/Explorer cut-and-paste is all-or-nothing and silently drops metadata. FTP clients queue, but do not verify or remove.

## 4c. Provenance journal ("git for where files went") (DECIDED: core, not optional)

Requirements from the user: tiny, fast, and able to say where every file ended up.

### Model
- **File identity** = content hash (BLAKE3). A file that moves keeps its identity; a file that is edited becomes a new identity with a `derived_from` edge if we saw the edit happen in place.
- **Op record** = `{ id, ts, folder, link, plan_id, kind, src, dst, hash, size, status, meta_lost[] }`. Append-only. Never updated except `status`.
- Storage: SQLite table with indexes on `hash`, `src`, `dst`, `ts`. "Tiny and fast" is satisfied by SQLite with WAL; a million ops is a few hundred MB and any lookup is an index hit. Do not build a custom log format until SQLite is proven too slow, which it will not be.
- Scope: **one global journal per machine** so `whereis <hash>` works across folders and links (a drain from folder A to folder B is one story). Per-folder export for portability.

### Schema versioning — **DECIDED (slice 4b): a newer file is refused, not skipped**

Reversal of the original behaviour, which silently skipped migration when the file's `user_version` exceeded what the binary knew, so an old binary could still read a newer journal. That was safe while every stored link end was a local absolute path: an old binary reading `/Volumes/nas/inbox` got the right answer even without understanding the columns beside it.

Schema v4 makes a link end *a connection plus a path relative to it*, with `NULL` meaning the local filesystem. An old binary reading a v4 row would see `inbox` and take it for a local relative path — and drain into it. `migrate` now returns `JournalError::TooNew { found, known }`.

Consequence to live with: a downgrade is not supported once a journal has been opened by a newer build. Given that the journal is the record of where every file went, refusing to open beats guessing.

### Queries (the git-like UX)
```
tungstate log <path|hash>        every op ever applied to this identity, oldest first
tungstate whereis <path|hash>    current location(s) across all folders, or "trashed at …"
tungstate show <op-id>           one op in detail, including metadata that could not be preserved
tungstate log --folder x --since 1d
tungstate blame <dir>            for every file here: how and when did it get here (its Source)
```
The `{Source}` template variable and `undo` both read from this journal. Building it early makes everything downstream cheaper.

## 5. Duplicate handling

### Detection tiers
1. **Size match** (free, from index). Only size-equal files can be duplicates.
2. **Partial hash**: first and last 64 KiB. Eliminates most false candidates cheaply.
3. **Full BLAKE3.** Cached keyed by (file id, size, mtime); invalidated when either changes.
4. **Near-duplicate (roadmap, not v1):** perceptual hash for images (`image_hasher`), chromaprint for audio, simhash for text. Different UX because it's fuzzy — always `suggest`, never `enforce`.

Also detect duplicate *directories* (same multiset of child hashes), because "I copied the whole folder twice" is the common case.

### Which copy is canonical?
Ordered tie-break, configurable: already at canonical path > older `birthtime` > shorter path > lexically first. User can pin via a `.tungstate-keep` marker or the CLI.

### What to do with the others — **DECIDED: rich action set, `trash` default**
- `skip`: leave it, just report.
- `trash`: move the non-canonical copies to trash. **Default.**
- `replace`: the *new* arrival wins; the existing canonical file goes to trash and the newcomer takes its place. Useful for "re-exported the video, overwrite the old one."
- `newer-wins` / `larger-wins`: `replace` or `trash` chosen by comparing mtime or size.
- `hardlink`: replace extras with hard links (same device only). Space reclaimed; editing one edits both. Power users.
- `reflink`: copy-on-write clone (APFS `clonefile`, btrfs/XFS `FICLONE`, ReFS block clone). Independent files, shared blocks. Falls back to `trash` where unsupported.
- `quarantine`: park extras with a report.
- `keep-both`: rename with suffix, no dedup.

Exact-dupe actions are safe to `enforce`. Near-dupe actions (roadmap) are always `suggest`.

### Scope
- Within a folder: v1.
- Across folders (e.g. "this is already in my archive"): v2, needs a global hash index.
- Local vs remote: hashing a remote requires downloading; use backend-provided checksums (S3 ETag for single-part, SFTP `check-file` extension, WebDAV `getetag`) when available and mark the hash algorithm in the index so BLAKE3 and MD5 never get compared.

---

## 6. Backends

### The abstraction
```rust
#[async_trait]
trait Backend: Send + Sync {
    fn capabilities(&self) -> Capabilities;   // atomic_rename, server_side_copy, native_hash(Algo), watch, case_sensitive, max_path_len, supports_hardlink, supports_reflink
    async fn list(&self, path: &Path, depth: Depth) -> BoxStream<Entry>;
    async fn stat(&self, path: &Path) -> Result<Meta>;
    async fn read_range(&self, path: &Path, range: Range<u64>) -> Result<Bytes>;
    async fn read(&self, path: &Path) -> Result<BoxAsyncRead>;
    async fn write(&self, path: &Path, body: BoxAsyncRead) -> Result<()>;
    async fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    async fn remove(&self, path: &Path) -> Result<()>;
    async fn mkdir(&self, path: &Path) -> Result<()>;
    fn watch(&self) -> Option<BoxStream<Event>>;
}
```
The planner consults `capabilities()` to decide between `rename` (atomic) and `copy+verify+delete`, and whether link-based dedup is even possible.

### Implementation strategy — **OPEN (Q3)**
- **OpenDAL (Apache):** one crate, 50+ services (fs, sftp, ftp, s3, gcs, azblob, webdav, gdrive, dropbox, onedrive, smb…), actively maintained, async. Downside: object-store-shaped; rename on some services is copy+delete under the hood; per-service quirks leak. You'd wrap it in the trait above and fill capability flags per scheme.
- **Hand-rolled per backend** (`russh`+`russh-sftp`, `suppaftp`, `aws-sdk-s3`, `reqwest` for WebDAV): full control, precise semantics, more code to own.
- **rclone as subprocess** (`rclone rcd` JSON-RPC): 70+ backends for free, battle-tested. Downside: external binary dependency, IPC latency, not "a single Rust binary."

Recommendation: **OpenDAL behind your own trait**, with `local` implemented natively (you need inode/file-id, xattrs, hardlink, reflink, and `notify`, none of which OpenDAL gives you). Hand-roll a backend only when OpenDAL's semantics prove wrong for it.

### SFTP is the exception — **DECIDED (slice 4b): hand-rolled on `russh` + `russh-sftp`**

Investigated while building slice 4b, before writing any SFTP code. OpenDAL's SFTP service is:

- **Unix-only.** It does not build on Windows, and tungstate promises three platforms.
- **Refusing of password authentication.** Key-based only. A consumer NAS is very often password-only, and telling the user to set up key auth first is not a product.
- **Delegating host-key checking to the system `ssh` client.** Tungstate wants trust-on-first-use against `~/.ssh/known_hosts` that it controls and can explain, not whatever the local `ssh` happens to be configured with.

Three requirements, three contradictions, so this is the escape hatch the paragraph above sanctions. SFTP will be built directly on `russh` + `russh-sftp` behind the same `Backend` trait. Everything else stays on OpenDAL. Implementation is much later than slice 4b; only the decision is recorded here so nobody re-investigates it.

### The async bridge — **DECIDED (slice 4b): spawn, never `block_on`**

OpenDAL has been async-only since 0.54 (RFC-6189). Its `opendal::blocking::Operator` is a wrapper around `Handle::block_on`, which **panics when the calling thread is already inside a tokio runtime** — and the GUI's Tauri commands are exactly such a thread. `tungstate-backend-opendal` therefore owns a `LazyLock<tokio::runtime::Runtime>` and dispatches each call as `handle.spawn(future)` followed by a blocking receive on a `std::sync::mpsc` channel. `spawn` is legal from any thread; `block_on` is not. This is §7's "adapter at the boundary", and it keeps every other crate synchronous exactly as §7 decided.

**Traps found while building, recorded so nobody re-introduces them:** `Operator::stat("/")` is short-circuited by OpenDAL to a synthetic directory without touching the store at all. Using it for a reachability check would silently disable the safety rail in §4b ("notice the volume changed underneath us"). A backend rooted at a local directory must `stat` that directory itself; anything else must list.

Slice 4c added three more, all in `opendal-service-ftp` 0.59.1. It answers **`rename` and `copy` with `Unsupported`** even though FTP has `RNFR`/`RNTO` and `suppaftp` exposes them, which is why the engine needed a commit path that does not rename. Its **writer streams to `<basename>.<8 random chars>` and renames on close**, so a write is atomic after all — but the random name means a crash leaks a file nothing can ever find, which is why recovery sweeps. And **every ranged read is a fresh `REST` + `RETR` data connection**, so a reader must stream once per file rather than per chunk; a 1 MiB ranged loop would open four thousand connections for a 4 GB file.

**The root of an FTP connection is an absolute server path, and OpenDAL `CWD`s to it.** A root of `/` therefore means the server's real root, not the directory the login lands in — on a typical server the account cannot write there at all. `connection test` prints the root it checked for exactly this reason.

### Two remote use-cases — **DECIDED: both in v1**
1. **Govern in place:** the remote *is* the folder; tungstate reorganises within it. Needs: remote listing, remote rename (or copy+delete), tiered attribute fetching (range reads for sniffing), remote-native checksums.
2. **Link (drain / ingest, §4b):** a remote is one end of a transfer link; classification happens on whichever end is a folder.

Both sit on the same `Backend` trait, so the extra cost of "both in v1" is mostly in the planner's handling of non-atomic rename and in polling. Build order within v1: local folder → drain link (local→remote) → ingest link (remote→local) → govern remote in place. Each step reuses the previous one's backend code.

**Network mounts (SMB/NFS/AFP) look local but are not.** Detect them (`statfs` `MNT_LOCAL` on macOS, `/proc/mounts` on Linux, `GetDriveType` on Windows) and treat as `local` backend with capabilities downgraded: no hardlink/reflink, `readback` verify, no watcher trust. A NAS mounted in Finder is the most common "remote" and it will be the first one tested.

### Platform specifics for `local`
- macOS: Unicode normalisation (NFD on HFS+, preserved-but-insensitive on APFS). Normalise to NFC for comparison, never rewrite the user's bytes. Dataless iCloud files (`SF_DATALESS`): never hash them, it triggers a download. `.DS_Store` ignore. `clonefile(2)` for reflink. Bundles.
- Windows: `\\?\` long-path prefix, case-insensitive-but-preserving, reserved names (`CON`, `NUL`), trailing dots/spaces, junctions vs symlinks, locked files (`ERROR_SHARING_VIOLATION` → retry with backoff, then skip). OneDrive placeholder files (same trap as iCloud). `FSCTL_DUPLICATE_EXTENTS_TO_FILE` on ReFS for reflink.
- Linux: inotify watch-descriptor limits (`max_user_watches`), `FICLONE` on btrfs/XFS, xattr availability varies.
- All: case-sensitivity is per-filesystem, not per-OS. Probe it at folder init by creating `a`/`A` in `.tungstate/` and record it in the index.

---

## 6b. Nodes: tungstate on every machine (DECIDED)

The deployment model the user described: tungstate is installed on the Mac (governs the Mac, drains to the NAS) **and** on the NAS (governs the NAS, receives drains from any device, self-governs). Each install is a **node**: one daemon, its own folders, links, journal, and index. Nodes are independent; nothing is centralised.

Consequences:
- **Governing over FTP is demoted.** The NAS governs its own disks locally through the `local` backend, which is faster, safer (atomic rename, hardlink, reflink), and needs no protocol quirks. Govern-in-place over a remote (slice 13) stays for appliances that cannot run software, at low priority.
- **A peer transfer protocol becomes the best drain (v0.3, slice 10b).** When both ends are tungstate, the laptop pushes over the daemon's HTTP API: `PUT /receive/{link}/{file-id}` with `Content-Range` for resume, the NAS hashes on receipt and returns the hash, the laptop compares to its stream hash, the NAS commits atomically into the destination folder's inbox and journals `origin = laptop-node/link-name`. Verification is native and free, resume is exact (the receiver reports its committed offset), metadata travels as a JSON sidecar in the same request, and `{Source}` works across machines. `tungstate://nas.local/videos` becomes a destination URL like any other. Mount and FTP drains remain for the v0.1 evacuation and for non-tungstate destinations.
- **Discovery and trust.** mDNS/DNS-SD advertisement (`_tungstate._tcp`) so `tungstate node list` shows the NAS, plus a one-time pairing token so only paired nodes can push. Never an open receive endpoint.
- **Builds.** Linux targets `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` (static, run on Synology/QNAP/TrueNAS without glibc worries), a Docker image for NAS app stores, `cross` in CI. macOS universal binary. Windows x86_64.
- **Cross-node journal queries** (`whereis` asking every paired node) are a v0.4 nicety; the per-node journal already records the destination node and path at drain time, so `whereis` on the laptop answers "on nas.local at /videos/2026/03/…" from local data alone.

## 7. Process model and architecture

### Binary layout — **DECIDED: API-first daemon, thin clients**
Because the user wants CLI, TUI, and a Web GUI, the daemon must expose one API that all three consume. Otherwise three UIs means three copies of the logic.

- One binary, `tungstate`. `tungstate daemon` runs folders, links, watchers, and an HTTP+JSON API (`axum`) bound to localhost (plus a Unix socket / named pipe for the CLI, same router). WebSocket or SSE endpoint streams events (progress, drift, ops) for live UIs.
- CLI subcommands are API clients when a daemon is running and fall back to running the engine in-process when it is not. Standalone mode is what makes `tungstate plan` usable without installing anything.
- TUI (`ratatui`) and Web GUI are pure API clients. The web assets are embedded in the binary (`rust-embed`) so the install story stays "one file."
- Auth: localhost-only bind plus a per-install token file, so a rogue browser tab cannot drive it. Optional LAN bind for controlling the NAS-side daemon from the laptop later.

`tungstate-api` crate holds the request/response types shared by server and all clients, so a schema change breaks at compile time in every client.

IPC transport detail: Unix domain socket on macOS/Linux, named pipe on Windows (`interprocess` or `tokio` + `hyper` over the socket). Same axum router either way.

Service install: `tungstate service install` writes a launchd plist / systemd user unit / Windows service (`service-manager` crate). Daemon must survive sleep/wake and network drops for remote folders.

### Crate workspace
```
tungstate/
  crates/
    tungstate-core/       # policy model, classifier, planner, journal — NO I/O, fully unit-testable
    tungstate-index/      # SQLite index
    tungstate-backend/    # Backend trait + local impl
    tungstate-backend-opendal/  # remote impls via OpenDAL, feature-gated per service
    tungstate-transfer/   # durable drain/ingest engine (§4b), per-file state machine, resume, verify
    tungstate-journal/    # provenance journal (§4c) — own crate because everything depends on it
    tungstate-api/        # shared request/response/event types for daemon and all clients
    tungstate-daemon/     # reconciliation loops, links, watchers, axum API server
    tungstate-cli/        # clap CLI, API client with in-process fallback, human + JSON output
    tungstate-tui/        # ratatui client
    tungstate-web/        # web client assets, embedded via rust-embed
  policies/               # built-in templates, include_str!'d
```
`tungstate-core` having zero I/O is the load-bearing decision for testability. Everything it needs comes in as data (`Snapshot`) and goes out as data (`Plan`).

### Config and state locations
`directories` crate for platform-correct paths. Global config `~/.config/tungstate/config.toml` lists folders. Per-folder policy at the root (local) or in the config dir (remote).

### Sync engine, async edge — **DECIDED (vetoable): the engine is synchronous**
The user is new to Rust. Async Rust (`Pin`, `Send + 'static` bounds, async traits, executor semantics) is the single biggest source of early pain, and nothing in the engine needs it: file I/O is blocking anyway, hashing is CPU-bound, and OpenDAL provides a blocking `Operator`.

- Engine crates (`core`, `transfer`, `journal`, `backend*`, `index`) are **sync**: `std::fs`, `std::thread`, `crossbeam-channel`, `rayon` for parallel hashing. The `Backend` trait is sync and object-safe (`Box<dyn Backend>`).
- Cancellation and pause are a shared `Arc<AtomicU8>` state plus a `crossbeam` channel checked between chunks. Progress is events on a channel. This is easy to reason about and easy to test.
- The daemon (slice 10) is the **only** async code: `axum` on `tokio`, calling the engine via `spawn_blocking` and forwarding engine events into WebSocket/SSE streams. By then the user has months of sync Rust behind them.
- If a future backend truly needs async (streaming from a cloud SDK with no blocking API), wrap it with a small runtime handle inside that backend, not by making the trait async. This is the "adapter at the boundary" pattern and it is what keeps the blast radius of async small.

`notify` is sync-native, so the watcher fits this model with no bridging.

**Would async be faster? (asked, answered: no, not for this workload.)** Throughput of a drain is bounded by disk bandwidth, network bandwidth, and BLAKE3 CPU time. None of those change with the concurrency model. Async wins when a process holds thousands of mostly idle connections (a web server); it does nothing for four busy file transfers. File I/O is blocking at the OS level on every platform, so `tokio::fs` is a thread pool underneath anyway. Threads with a bounded pool give true parallelism for hashing with no executor overhead. The one place async pays for itself is the daemon's HTTP layer holding open SSE connections to the TUI and web clients, and that is the one place we use it. Revisit only if a remote backend needs thousands of in-flight small requests (mass `stat` over SFTP), and even then a 16-thread pool will match it.

### Errors, logging, telemetry
`thiserror` in libraries, `miette` in the CLI (pretty diagnostics with policy-file spans so "line 14: ambiguous match between `Photos` and `Screenshots`" points at the line). `tracing` everywhere, `tracing-subscriber` with a JSON layer for the daemon. No network telemetry, ever; this tool reads people's files.

---

## 8. CLI and UX — **DECIDED: CLI, TUI, and Web GUI, all on the daemon API**

```
tungstate init [--template <name>] [path]     scaffold a policy
tungstate adopt <path>                        scan an existing folder, report drift, propose a policy from what's there
tungstate plan [folder] [--json]               compute and print the plan; no changes
tungstate apply [folder] [--yes] [plan.json]   apply a plan; circuit breaker applies
tungstate watch [folder]                       foreground reconciliation loop (daemon-lite)
tungstate daemon [start|stop|status]
tungstate status                              drift summary per folder
tungstate explain <file>                      why is this file here / where would it go, full decision trace
tungstate dedupe [folder] [--strategy ...]     dedup pass
tungstate undo [--last N | --plan <id>]
tungstate log <path|hash>                     provenance history (git-log feel)
tungstate whereis <path|hash>                 where is this file now, across all folders
tungstate blame <dir>                         per file: how and when did it arrive here
tungstate link add <from> <to> [--drain|--ingest] [--remove-source] [--verify readback]
tungstate link start|pause|resume|status <name>   durable transfer control, progress, space reclaimed
tungstate connection add|list|test|remove <name>  places a link end can live, other than this machine
tungstate connection update <name> [--host ...]   change one; the name itself cannot change
tungstate connection password <name>              replace the stored password (the recovery path for a refused sign-in)
tungstate quarantine [list|release|resolve]
tungstate doctor                              check watcher limits, FS capabilities, policy validity
tungstate policy [validate|fmt|explain]
tungstate service [install|uninstall]
```

Every command takes `--json` for scripting. `adopt` is the killer onboarding feature: point it at a messy folder and it proposes a policy inferred from the existing structure and the file mix, then shows what would move.

Desktop notifications (`notify-rust`) in `suggest` mode: "12 files in Downloads need sorting. `tungstate apply downloads` or open plan." Never notify in `enforce` mode except on quarantine or circuit-breaker trips. Drain links notify on: completed, paused-unreachable for more than an hour, failed file needing a decision. Nothing else; a notification per file is spam.

### Connections are a first-class screen — **DECIDED (slice 4e)**

The desktop app has a **Connections** tab alongside Browse, Transfers and
Activity: add, edit, test, change password, forget. A connection then appears
in each pane's "Go to…" list as `name:`, and a pane can browse it and drain
into it whether or not the operating system has mounted anything. That is the
point — the FTP route exists precisely for when the mount does not work, and
until this screen existed it was reachable only from the command line.

A pane's location is one composite string, `/Users/me/Videos` or
`nas:inbox/2026`, read by the same parser the command line uses
(`tungstate-journal::ends`). One grammar rather than two, because the address
bar, the "Go to…" list, the saved pane state, a transfer leg and a link spec
are all a single string already. A name of two or more characters before a
colon is a connection; one character is a Windows drive letter. No drive letter
is ever two characters, so the two cases cannot collide.

**Rename is deliberately unsupported.** A connection's name is both the token
people typed into their link specs and the account its password is filed under
in the keychain, so renaming is two migrations wearing one hat — the journal
rows and the platform credential store, which can fail independently. The
journal's `ConnectionSettings` has no `name` field at all, so the API cannot
express a rename and therefore cannot desynchronise the two. Remove and re-add
is the supported path, and it is refused while a link still points at the
connection, which is the prompt to fix the links first.

Editing everything else *is* supported while links point at a connection, and
that asymmetry is the reason connections exist as an indirection: a link holds
a connection id, so moving the connection's host or root re-points every link
at once. Deleting would leave those links pointing at nothing, which is what
the foreign key refuses.

### `adopt` (policy inference, v0.4, design intent now)
Scan the folder, cluster files by mime and by the directory they already sit in, and propose `[[rule]]` entries that would keep most files where they are. Report the fit ("proposed policy explains 94% of files, 312 would move") and let the user edit before applying. It never applies on its own. Inference is heuristic; the point is a good starting file, not a perfect one.

### Leave-a-stub option (drain, roadmap)
Some users want to see in Finder that a file was drained and where. `--stub` leaves a tiny `name.ext.tungstate` file with the destination and hash, or a symlink when the destination is a mount. Off by default; `whereis` is the primary answer.

### LAN-exposed daemon auth (v0.3, with nodes)
Localhost binds need only the per-install token file. LAN binds require pairing: a short-lived code shown on one node, entered on the other, exchanging long-lived tokens. TLS with a self-signed per-node certificate pinned at pairing time, so a compromised network cannot inject files into a drain. No passwords, no accounts.

TUI (`ratatui`): folder list, link progress (files left, bytes left, space reclaimed, ETA, paused-because-unreachable), live drift, plan review with per-op approve/reject. The drain progress screen is the TUI's reason to exist.

Web GUI: same screens, reachable from another machine on the LAN later (watch the drain from your phone). **DECIDED:** Vue 3 + Vite, kept small (no Nuxt, no state library until needed), built to static files and embedded in the binary with `rust-embed`. The daemon API is plain JSON over HTTP plus an SSE event stream, so the TUI and the CLI use the exact same endpoints.

---

## 9. Edge cases and traps (collected, so none are forgotten)

- Two files with the same canonical path arriving in the same cycle (e.g. two photos taken in the same second): the rename template must include a disambiguator (`{hash:8}`) or the planner appends `-1`, `-2`.
- Policy change that retroactively moves everything: circuit breaker + `tungstate plan` shows the migration. Consider `tungstate policy diff` that shows how placement changes for a sample.
- Nested or overlapping governed folders: refuse at config load. One governed folder cannot contain another's root. (Revisit if a real need appears.)
- Folder root on a removable drive or network mount that disappears: daemon marks folder `unavailable`, keeps the index, resumes on reappearance; never plans against a partial view.
- Disk full mid-copy: verify step fails, source is kept, op is journaled as failed, folder pauses with a notification.
- Clock skew on remotes: never use remote mtime for ordering decisions across backends; only within one backend.
- Timezone for date-templated paths: EXIF times are local-naive. Policy declares `tz = "local" | "utc" | "Africa/Lagos"`.
- Files being written by another process at plan time: cooldown + pre-op re-stat.
- Hard links already present in the tree: the index records file id; two paths with one id are one file, not a duplicate.
- Symlinks inside the folder: policy `symlinks = "ignore" | "treat-as-file" | "follow-within-folder"`. Default `ignore`.
  - **Implemented in slice 1 as refusal.** `LocalBackend::guarded` walks each path component and rejects if any is a symlink, because `resolve` sees only path text and a link's name reveals nothing about its target. Operations that act on the link itself (`rename`, `remove_*`, `stat`) still permit a link as the final component.
- **The destination's identity is pinned for the length of a run.** A backend reports a `RootToken` for its root, captured before the first file and compared before every subsequent one. On Unix that carries the filesystem's device id, so an unmounted NAS whose mount point survives as an empty directory on the boot disk is still detected. Windows has no equivalent on stable Rust, so it degrades to a reachability check.
  - **Why it exists:** without it, `create_dir_all` recreated a vanished mount point and the drain wrote to the boot disk it was supposed to be emptying, deleting each original after "verifying" the copy. Observed, not theorised: 572 MB landed on the wrong disk and five originals were deleted.
  - A run that loses its destination stops, reports it, and prunes nothing. Every untouched original stays where it is.
  - **Known gap:** the check is not atomic with the open that follows, so an attacker able to swap a directory for a symlink in that window could still escape. Closing it needs `openat`-style per-component opens and therefore `unsafe`, which the workspace lints forbid. Revisit if tungstate is ever pointed at a directory writable by an untrusted party.
- Sparse and very large files: partial hash first; full hash only when partial matches; stream, never read into memory.
- Path length limits on Windows and on remotes (FTP servers with 255-byte limits): capability flag, planner validates generated paths and quarantines with a reason instead of failing the op.
- Permission denied on a subtree: log, mark subtree `unreadable` in index, continue. Never abort the whole cycle for one bad directory.
- Filename characters illegal on the *target* backend (`:` on Windows, `/` in names from S3 keys): rename template gets a `sanitize` filter; default on.

---

## 10. Testing strategy

- **Property tests** (`proptest`) on `tungstate-core`: generate random trees + random policies, assert (a) applying the plan then re-planning yields an empty plan, (b) content-hash multiset is preserved, (c) no op targets a path outside root, (d) plan DAG has no cycles after temp-name breaking.
- **Snapshot tests** (`insta`) for `explain` output and plan rendering, so UX regressions are visible in review.
- **In-memory fake backend** implementing the trait, used by all planner tests. Fault injection: fail the Nth op, drop events, return stale stats.
- **Integration tests** on real temp dirs (`tempfile`) for the local backend, including a case-insensitive FS check (macOS default) and a real `notify` watcher test with generous timeouts.
- **Remote backends** tested against Docker containers (`sftp`, `vsftpd`, `minio`) in CI, feature-gated so `cargo test` stays fast locally.
- Parallel-safe by construction: every test gets its own temp dir and its own SQLite file; no fixed ports (containers get random ports via `testcontainers`).

---

## 11. Build order (a syllabus, since the user is building it)

Each slice is small enough to finish in days, ends in something runnable, and teaches one or two Rust concepts. **DECIDED: drain first.** The user is new to Rust, so slices 0–9 are entirely synchronous; `tokio` first appears in slice 10.

### How a slice runs (the teaching loop)
1. Claude writes a **slice brief**: goal, the runnable outcome, the Rust concepts involved with a short explanation of each, the crates to pull in and why those, the module and type shape suggested, acceptance criteria, and the tests that must pass.
2. The user builds it and shares the diff or pushes a branch.
3. Claude reviews for correctness, idiom, and design, explaining every correction (what a senior Rust engineer would do, and why). Corrections are suggestions unless they are bugs.
4. Repeat until acceptance criteria pass, then the next brief.
Claude takes over a slice only when the user names it explicitly.

The "Teaches" column below is adjusted for a Rust newcomer: ownership and borrowing in slice 1, `Result` and error design in slice 1–2, traits and trait objects in slice 1, enums as state machines in slice 3, threads and channels in slice 3–4, lifetimes when the parser needs them in slice 5.

| # | Slice | Runnable outcome | Teaches |
|---|---|---|---|
| 0 | Workspace skeleton, `tungstate-api` types, CI, `cargo clippy -D warnings`, `insta` + `proptest` wired | `tungstate --version` | workspace layout, feature flags, error types (`thiserror`), `tracing` |
| 1 | `Backend` trait + `local` impl + network-mount detection | list/stat/read/write on a temp dir with tests | ownership and borrowing, `Result` and `thiserror`, traits and `Box<dyn Trait>`, platform `cfg` |
| 2 | Journal crate (SQLite, WAL, write-ahead op records) + `log`/`whereis` | `tungstate log <file>` on a hand-inserted row | `rusqlite`, migrations, indexes, newtype IDs, `From` conversions |
| 3 | Transfer engine: per-file state machine, temp name, streaming BLAKE3, three verify levels, commit, remove source | **`tungstate link add ~/Videos /Volumes/nas/inbox --drain --remove-source` works, resumable** | enums as state machines, `Read`/`Write` streaming, `std::thread`, `crossbeam-channel`, crash-recovery tests with fault injection |
| 4 | Reachability pause/resume, ordering strategies, concurrency, bandwidth cap, progress events | close the lid, walk away, come back, it continues | `Arc`, atomics, backoff, worker pools, `Mutex` vs message passing |
| 5 | Policy model + parser + `explain` (TOML, templates, buckets, precedence rules, load-time ambiguity errors) | `tungstate explain <file>` prints the decision trace | `serde`, parsing, `miette` diagnostics with spans, property tests |
| 6 | Index + scan + planner (pure, no I/O) + `plan` | `tungstate plan` on a messy folder | DAG/topo-sort, cycle breaking, `proptest` invariants |
| 7 | Executor + circuit breaker + `apply` + `undo` via journal | first real reorganisation, and its reversal | idempotency, journaled execution |
| 8 | Dedup (size → partial → full hash, canonical selection, all actions) | `tungstate dedupe` | hashing pipelines, `rayon`, cache invalidation |
| 9 | Watcher + cooldown + self-event suppression + periodic rescan + `watch` | foreground continuous governance | `notify`, debouncing, channels bridging sync/async |
| 10 | Daemon: axum API, SSE event stream, service install, CLI-as-client with fallback | set-and-forget on the laptop | first async: `tokio`, `axum`, `spawn_blocking` bridging to the sync engine, `launchd`/`systemd`/Windows service |
| 10b | Nodes: Linux musl builds, pairing, peer transfer protocol (`tungstate://` destination), mDNS discovery | drain laptop → NAS with native verification and exact resume, NAS self-governs | HTTP range semantics, TLS pinning, cross-compilation |
| 11 | TUI: link progress, folder drift, plan review | watch the drain in a terminal | `ratatui`, event loops, API client |
| 12 | OpenDAL backends (SFTP, FTP, S3, WebDAV) + ingest links + remote polling | FTP drop folder ingests into the laptop folder | adapter pattern, capability flags, integration tests with `testcontainers` |
| 13 | Govern remote folders in place (tiered attribute fetch, non-atomic rename planning, remote trash) | reorganise the NAS over SFTP | cost-aware planning |
| 14 | Web GUI | phone-viewable drain progress | chosen web stack |
| 15+ | `adopt`, near-dupes, directory dedup, cross-folder dedup, Rhai classifiers, Wi-Fi-aware run conditions | | |

**DECIDED (NAS is reached via Finder mount and via FTP):** the first drain (slice 3) targets the Finder-mounted volume through the local backend with network-mount detection. An extra slice **4b** follows immediately: OpenDAL FTP backend (blocking API), `Capabilities` flags for FTP (resume via `REST`, no native checksum, non-atomic rename on some servers), and the same drain over `ftp://`. Slice 12 then shrinks to SFTP, S3, and WebDAV. FTP has no server-side checksum, so `readback` is the recommended (not default) verify level for FTP links, and the CLI says so when such a link is created.

Slices 0–4b are "v0.1: the drain works and I trust it, over a mount and over FTP." Slices 5–9 are "v0.2: governance." 10–14 are "v0.3: daemon and UIs."

**Licence and hosting (DECIDED):** `MIT OR Apache-2.0`, public GitHub repository, Rust ecosystem convention.

---

## 12. Decisions so far

- **DECIDED** Hybrid model, two skins (§1). TOML policy (§2). SQLite index and journal (§3, §4c). OpenDAL behind own trait (§6). Both drain/ingest and govern-in-place in v1 (§6). Rich dedup action set, `trash` default (§5). API-first daemon with CLI, TUI, Web GUI as clients (§7, §8). User builds, Claude guides (top).
- **DECIDED** Drain first (§11). Verify levels `size` / `hash` / `readback`, default `hash` (§4b). Vue 3 web GUI, small, embedded (§8). User is new to Rust, strong elsewhere; sync engine, async only in the daemon (§7).
- **DECIDED** NAS via Finder mount and FTP; FTP backend becomes slice 4b (§11). Noun is `folder` (§1). `MIT OR Apache-2.0` on GitHub (§11). Sync engine confirmed, with the performance reasoning recorded (§7).
- **DECIDED** Tungstate runs on every machine as a node; NAS self-governs; peer transfer protocol in v0.3 (§6b). Source handling is an explicit `--move` / `--move --trash` / `--copy` choice per link, no silent default (§4b). `[[rule]]` noun; policy at `<folder>/.tungstate/policy.toml` (§2). Rename filters, `adopt` intent, notification rules, stub option, LAN auth (§2, §8).

Every part has now been brainstormed at least once. Nothing remains OPEN.

---

## 13. What happens after this document is approved

Because the user builds and Claude guides, "execution" of this plan is documentation and a first brief, not code:

1. `git init` the repo, add `LICENSE-MIT`, `LICENSE-APACHE`, a `.gitignore` for Rust and Node.
2. Write `docs/DESIGN.md` from §1–§10 (the spec) and `docs/SYLLABUS.md` from §11 (the build order and teaching loop), so the design lives in the repo, not in a chat.
3. Write `docs/slices/00-skeleton.md`: the first slice brief (workspace layout, crate names, `tungstate-api` placeholder types, CI with `clippy -D warnings` and `cargo fmt --check`, `insta` and `proptest` wired, `tungstate --version`), with acceptance criteria.
4. Save the working-mode agreement (user builds, Claude is PM and senior Rust engineer, hands over only named slices) to Claude's memory so it survives future sessions.
5. The user builds slice 0. Claude reviews.

No application code is written by Claude unless a slice is explicitly handed over.
