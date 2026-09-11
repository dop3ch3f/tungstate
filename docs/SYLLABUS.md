# Tungstate Build Syllabus

Extracted from `DESIGN.md` §11. Each slice gets its own brief under `docs/slices/`.

## 11. Build order

Each slice is small enough to finish in days, ends in something runnable, and teaches one or two Rust concepts. **DECIDED: drain first.** The user is new to Rust, so slices 0–9 are entirely synchronous; `tokio` first appears in slice 10.

### How a slice runs (changed 2026-09-09)

Originally the user wrote the code and Claude reviewed. Writing Rust as a newcomer proved to be the slow step, and the user is shipping a product, so that step moved to Claude. Everything the user is fast at stayed with them.

1. Claude writes a **slice brief**: goal, runnable outcome, the design decisions and their alternatives, acceptance criteria, and the tests that must pass. The brief is the spec.
2. Claude implements it, and does not stop until `build`, `test`, `clippy -D warnings`, `fmt --check` and CI on all three platforms are green.
3. Claude writes a **tour** at `docs/slices/NN-tour.md` walking the shipped code and explaining each Rust concept where it appears. Source comments stay short and load-bearing; the teaching lives in the tour.
4. The user reads the tour and the diff, then says go. Claude stops after every slice.

Claude still asks before choosing on anything non-obvious. No silent design calls.

The "Teaches" column below is adjusted for a Rust newcomer: ownership and borrowing in slice 1, `Result` and error design in slice 1–2, traits and trait objects in slice 1, enums as state machines in slice 3, threads and channels in slice 3–4, lifetimes when the parser needs them in slice 5.

| # | Slice | Runnable outcome | Teaches |
|---|---|---|---|
| 0 | [Workspace skeleton](slices/00-skeleton.md), `tungstate-api` types, CI, `cargo clippy -D warnings`, `insta` + `proptest` wired | `tungstate --version` | workspace layout, feature flags, error types (`thiserror`), `tracing` |
| 1 | [`Backend` trait + `local` impl](slices/01-backend-trait.md) — [tour](slices/01-tour.md) | list/stat/read/write on a temp dir with tests | ownership and borrowing, `Result` and `thiserror`, traits and `Box<dyn Trait>`, capability probing |
| 2 | [Journal crate (SQLite, WAL, write-ahead op records) + `log`/`whereis`](slices/02-journal.md) — [tour](slices/02-tour.md) | `tungstate log <file>` and `whereis` on a real database | `rusqlite`, modules and visibility, migrations, `Mutex`, unsigned/signed boundaries |
| 3 | [Transfer engine: durable drain, verify levels, conflicts, resume](slices/03-drain.md) — [tour](slices/03-tour.md) | **`tungstate link add ~/Videos /Volumes/nas --name x --move` then `link run x`, survives kill -9** | struct lifetimes, `&mut dyn Trait`, `Read`/`Write` streaming, declarative macros, crash-recovery testing |
| 4 | [Concurrency, sensed not configured](slices/04-concurrency.md) — [tour](slices/04-tour.md): several files at once, the limit found by handshake before the first file, back-off if the far side objects. Reachability pause/resume and bandwidth caps follow separately | **2.4× on 300 small files over FTP, with no file failing to calibration** | scoped threads, `Mutex` to turn `&mut self` into `&self`, `Send` bounds, races concurrency creates |
| 4b | [The connection seam](slices/04b-connections.md) — [tour](slices/04b-tour.md): a link end becomes a connection plus a path, journal v4, `tungstate-secret`, `tungstate-backend-opendal` with `fs`/`memory` | **`tungstate connection add scratch --scheme fs --root /tmp/x` then `link add ~/files scratch:inbox --move` and `link run`** — the whole drain over OpenDAL, no network | driving async from sync without `block_on`, `Box<dyn Trait>` from a fallible factory, `Option<Id>` vs an enum variant in a schema, `Path::components()` vs `join`, a trait with a fake for tests |
| 4c | [FTP](slices/04c-ftp.md) — [tour](slices/04c-tour.md): `services-ftp` and `ftps`, the engine's no-rename commit path, one streamed read per file, the orphan sweep, Docker integration tests on Linux CI | **a real drain to the NAS over FTP, killed mid-file and resumed** | capability flags that change behaviour, streams bridged to `io::Read`, reading a dependency's source when it lies to you |
| 4d | [The window and the run](slices/04d-window-and-run.md) — [tour](slices/04d-tour.md): interrupted work found across links and offered with Resume or Clean up | **kill the app mid-drain, reopen, finish or clear what it left** | `Drop` as a guarantee, sharing a destructive rule, knowing when to back a feature out |
| 4f | [The queue is a real thing](slices/04f-the-queue.md) — [tour](slices/04f-tour.md): a link records what it was asked to move (v5), the plan is announced before the run, per-file byte progress | **tick files, kill mid-run, reopen: the batch is counted and Resume takes only the batch** | additive schema as a default, trait methods with defaults, throttling where the knowledge is |
| 4g | [A stop that means stop, and a limit that can come back down](slices/04g-stopping-and-sensing.md) — [tour](slices/04g-tour.md): a kill switch honoured mid-chunk, the comparison phase reported as its own thing, throughput-measured ramp-down, asking before removing an original that is already over there | **press Stop now mid-file and get Resume or Clean up; watch "4 at a time" fall to 2 on its own** | `AtomicU8` as a state machine, `Condvar` and `wait_timeout`, `impl FnMut` as a progress reporter, failing in a shape recovery already knows |
| 4e | Connections in the desktop app: a fourth tab, an add-connection dialog, connections in the pane's "Go to…" list, the dual-pane browser reading a remote | add a NAS in the window and drag files onto it | Tauri commands over a fallible factory, TypeScript mirrors in `ui/src/api.ts` |
| 5 | Policy model + parser + `explain` (TOML, templates, buckets, precedence rules, load-time ambiguity errors) | `tungstate explain <file>` prints the decision trace | `serde`, parsing, `miette` diagnostics with spans, property tests |
| 6 | Index + scan + planner (pure, no I/O) + `plan` | `tungstate plan` on a messy folder | DAG/topo-sort, cycle breaking, `proptest` invariants |
| 7 | Executor + circuit breaker + `apply` + `undo` via journal | first real reorganisation, and its reversal | idempotency, journaled execution |
| 8 | Dedup (size → partial → full hash, canonical selection, all actions) | `tungstate dedupe` | hashing pipelines, `rayon`, cache invalidation |
| 9 | Watcher + cooldown + self-event suppression + periodic rescan + `watch` | foreground continuous governance | `notify`, debouncing, channels bridging sync/async |
| 10 | Daemon: axum API, SSE event stream, service install, CLI-as-client with fallback | set-and-forget on the laptop | first async: `tokio`, `axum`, `spawn_blocking` bridging to the sync engine, `launchd`/`systemd`/Windows service |
| 10b | Nodes: Linux musl builds, pairing, peer transfer protocol (`tungstate://` destination), mDNS discovery | drain laptop → NAS with native verification and exact resume, NAS self-governs | HTTP range semantics, TLS pinning, cross-compilation |
| 11 | TUI: link progress, folder drift, plan review | watch the drain in a terminal | `ratatui`, event loops, API client |
| 12 | WebDAV and S3 backends + ingest links + remote polling (FTP landed in 4c; SFTP is hand-rolled on `russh`, see DESIGN.md §6) | a WebDAV drop folder ingests into the laptop folder | capability flags, integration tests with `testcontainers` |
| 13 | Govern remote folders in place (tiered attribute fetch, non-atomic rename planning, remote trash) | reorganise the NAS over SFTP | cost-aware planning |
| 14 | Web GUI | phone-viewable drain progress | chosen web stack |
| 15+ | `adopt`, near-dupes, directory dedup, cross-folder dedup, Rhai classifiers, Wi-Fi-aware run conditions | | |

**DECIDED (NAS is reached via Finder mount and via FTP):** the first drain (slice 3) targets the Finder-mounted volume through the local backend with network-mount detection. Slice **4b** then widens the seam — a link end becomes a connection plus a path inside it — and **4c** adds FTP itself, with `Capabilities` flags for FTP (no native checksum, non-atomic rename on some servers) and the same drain over `ftp://`. **4d** puts connections in the desktop app. Slice 12 then shrinks to WebDAV and S3, since FTP landed in 4c and SFTP is hand-rolled (DESIGN.md §6). FTP has no server-side checksum, so `readback` is the recommended (not default) verify level for FTP links, and the CLI says so when such a link is created.

Slices 0–4d are "v0.1: the drain works and I trust it, over a mount and over FTP." Slices 5–9 are "v0.2: governance." 10–14 are "v0.3: daemon and UIs."

**Licence and hosting (DECIDED):** `MIT OR Apache-2.0`, public GitHub repository, Rust ecosystem convention.



## Scope changes made during the build

- **Per-file resume rather than byte-offset, in slice 3.** Everything committed is kept and only the file in flight is redone. Byte-offset resume moves to slice 4c or later, where FTP makes partial transfers genuinely expensive; over a mounted volume the code to track and validate partial writes carries more corruption risk than the minutes it saves.
- **Slice 4f inserted (2026-09-11).** Found by using 4d rather than testing it. A link recorded its two roots and not which files the user picked, so a selection lived only in a worker thread and died with the process. Three symptoms, one cause: an interrupted batch could only report the one file that had started, a resumed run re-walked the whole source folder instead of the batch, and no queue could be shown because none existed outside a running process. Migration v5 stores the selection; "no rows" keeps meaning the whole root, so saved folder-pairs and every existing link are unchanged. "Progress events" moves out of slice 4 into here, leaving slice 4 as concurrency, bandwidth caps and reachability pausing.
- **Slice 4d inserted, connections moved to 4e (2026-09-11).** Unplanned, and found by using the desktop app rather than by testing it. A one-off browser transfer makes an unsaved link, `links()` hides unsaved links, and recovery is scoped per link — so an interrupted browser transfer was unreachable and its partial orphaned on the destination for ever. Two other bugs in the same session: Tauri resolved an empty ACL so the window could not hear any engine event, and the Activity view used five class names the stylesheet does not define. The lesson recorded: "compiles and launches" is not a done bar for anything with a screen.
- **Slice 4b split into three (2026-09-11).** What was one "OpenDAL FTP backend" slice became the seam (4b), FTP (4c), and the desktop app's connection screens (4d). The seam is the part that touches every crate and needs a schema migration, and it can be proven on all three CI platforms with no network at all by running the existing transfer suite through OpenDAL's own `fs` service. Shipping it first makes FTP a service registration plus its quirks rather than a rewrite.
- **SFTP leaves OpenDAL (2026-09-11).** OpenDAL's SFTP service is Unix-only, refuses password authentication, and delegates host-key checking to the system `ssh` client — contradicting three-platform support, password auth for a consumer NAS, and tungstate-controlled trust-on-first-use. It will be hand-rolled on `russh` + `russh-sftp` behind the same `Backend` trait. Recorded in DESIGN.md §6; implemented much later.
- **The engine gained a commit path that does not rename (slice 4c).** `Capabilities::atomic_rename` was defined in slice 1 and read by nothing until FTP, whose OpenDAL service answers `rename` with `Unsupported`. Without rename the only route to the final name is to write to it, so `copy_verify_commit` branches and recovery follows. `ConflictAction::Replace` is refused on such a destination, because moving the existing file aside is the promise it cannot keep. S3 and WebDAV need the same path in slice 12.
- **An old binary can no longer open a newer journal (2026-09-11).** Schema v4 makes a link end relative to a connection, so a build that does not know about connections would read a remote path as a local one. `migrate` returns `JournalError::TooNew` rather than skipping. Recorded in DESIGN.md §4c.
- **Links are stored in the journal database, not a config file.** A link's definition and its progress then update together, and a resumed drain finds its own interrupted work by link id. This widens the journal crate from provenance to persistent state.
- **Network-mount detection moved out of slice 1 into slice 4.** Reading `MNT_LOCAL` on macOS or `GetDriveType` on Windows means either `unsafe` FFI, which the workspace lints forbid, or a heavy dependency. It is also a poor second lesson in Rust. Nothing needs it until slice 4 picks a default verification level per link. Slice 1 instead discovers hard-link and case-sensitivity support by probing the filesystem, which is safe, portable, and more accurate than asking the operating system.
