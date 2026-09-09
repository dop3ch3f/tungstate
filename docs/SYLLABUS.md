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



## Scope changes made during the build

- **Network-mount detection moved out of slice 1 into slice 4.** Reading `MNT_LOCAL` on macOS or `GetDriveType` on Windows means either `unsafe` FFI, which the workspace lints forbid, or a heavy dependency. It is also a poor second lesson in Rust. Nothing needs it until slice 4 picks a default verification level per link. Slice 1 instead discovers hard-link and case-sensitivity support by probing the filesystem, which is safe, portable, and more accurate than asking the operating system.
