# Tungstate Build Syllabus

Extracted from `DESIGN.md` §11. Each slice gets its own brief under `docs/slices/`.

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

