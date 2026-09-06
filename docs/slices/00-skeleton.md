# Slice 0: Workspace skeleton

**Runnable outcome:** `tungstate --version` prints a version. `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` are green locally and in CI on macOS, Linux, and Windows.

**Why this slice exists:** every later slice adds a crate to this workspace and relies on the lints, test tooling, and CI being already in place. Getting them right now means no slice ever starts with "first fix the build."

## Concepts you will meet (new-to-Rust notes)

- **Crate vs module.** A crate is a compilation unit and a Cargo package (one `Cargo.toml`). A module is a namespace inside a crate (`mod foo;` or `foo.rs`). We split into crates for compile-time boundaries and for build parallelism, not for namespacing.
- **Workspace.** One root `Cargo.toml` listing member crates. All members share one `Cargo.lock` and one `target/` directory, and can share dependency versions and lint settings, which is why we set those once at the root.
- **`lib` vs `bin`.** `tungstate-api` is a library (`src/lib.rs`). `tungstate-cli` is a binary (`src/main.rs`) whose executable is named `tungstate`. Libraries hold logic and are testable; binaries are thin.
- **Why `tungstate-api` exists on day one.** It holds the types every client and the daemon will share (versions, IDs, request and response shapes). Starting it now, even nearly empty, makes "change a type, every client fails to compile" the default behaviour from the first commit.
- **`Result<T, E>` and `?`.** Rust has no exceptions. A function that can fail returns `Result`. The `?` operator returns early with the error if there is one. You will write `fn main() -> anyhow::Result<()>` in the CLI so `?` works at the top level.
- **Lints as policy.** `-D warnings` turns every warning into a build failure. `unsafe_code = "forbid"` makes `unsafe` a compile error across the workspace. Both are cheap to enable now and painful to enable later.
- **Edition 2024.** Editions are opt-in language revisions. Use the current one for a new project.

## Layout to produce

```
tungstate/
  Cargo.toml                  workspace root
  rust-toolchain.toml         pins the stable channel
  rustfmt.toml                formatting choices (keep tiny)
  .github/workflows/ci.yml
  crates/
    tungstate-api/
      Cargo.toml
      src/lib.rs
    tungstate-cli/
      Cargo.toml
      src/main.rs
      tests/cli.rs            end-to-end tests that run the built binary
```

Only these two crates. Later slices add `tungstate-backend`, `tungstate-journal`, `tungstate-transfer`, and the rest as they are needed. Empty placeholder crates are noise.

## Root `Cargo.toml` shape

```toml
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
version = "0.0.1"
edition = "2024"
license = "MIT OR Apache-2.0"
repository = "https://github.com/dop3ch3f/tungstate"
rust-version = "1.98"

[workspace.dependencies]
# Runtime
clap = { version = "4", features = ["derive"] }
anyhow = "1"            # CLI-only error type; libraries use thiserror (slice 1)
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde = { version = "1", features = ["derive"] }
# Test
insta = { version = "1", features = ["yaml"] }
proptest = "1"
assert_cmd = "2"

[workspace.lints.rust]
unsafe_code = "forbid"
missing_debug_implementations = "warn"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
must_use_candidate = "allow"
```

Each member crate opts in with `[lints] workspace = true` and pulls dependencies with `clap = { workspace = true }` so versions live in one place. Check crates.io for the current major versions before pinning; the numbers above are the ones expected to be current, verify them.

## What `tungstate-api` contains in this slice

```rust
//! Types shared by the daemon and every client (CLI, TUI, web).
//! Anything here is a wire contract: changing it must keep old clients working.

/// Version of the tungstate binary, from Cargo at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Strongly typed identifier for a governed folder.
/// A newtype so a folder id can never be passed where a link id is expected.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct FolderId(pub String);
```

That is all. The newtype pattern (`struct FolderId(String)`) is the idiom you will use for every id in the project; it costs nothing at runtime and catches argument-order bugs at compile time.

## What `tungstate-cli` contains in this slice

- A `clap` derive `Cli` struct with `#[command(version = tungstate_api::VERSION)]`.
- Subcommands `folder` and `link`, each with an `add` that prints `not implemented yet` and exits non-zero. They exist so the command shape from `DESIGN.md` §8 is visible from the first build.
- `tracing_subscriber` initialised from `RUST_LOG` so every later slice has logging without setup.

## Tests to write

1. `crates/tungstate-cli/tests/cli.rs`: run the binary with `--version` using `assert_cmd`, assert success and that stdout contains `tungstate_api::VERSION`.
2. Same file: `--help` output captured and compared with an `insta` snapshot. This makes any accidental change to the command surface show up in review as a snapshot diff.
3. `crates/tungstate-api/src/lib.rs`: a `proptest` that `FolderId` round-trips through `serde_json`. Trivial, but it proves proptest and serde are wired and shows you the shape of a property test.

## CI (`.github/workflows/ci.yml`)

Matrix over `ubuntu-latest`, `macos-latest`, `windows-latest`. Steps: checkout, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache`, `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-features`. Windows is in the matrix from day one because path handling bugs are cheaper to find early.

## Acceptance criteria

- [ ] `cargo build` succeeds on your Mac with zero warnings.
- [ ] `tungstate --version` prints `tungstate 0.0.1`.
- [ ] `tungstate folder add x` and `tungstate link add a b` print `not implemented yet` and exit with status 2.
- [ ] `cargo test` passes, including one `insta` snapshot and one `proptest`.
- [ ] `cargo clippy --all-targets -- -D warnings` is clean.
- [ ] `cargo fmt --check` is clean.
- [ ] CI is green on all three platforms.
- [ ] No `unwrap()` outside tests. Use `?` or `expect("reason")` with a reason that says why it cannot fail.

## How to hand it in

Push a branch and share the diff, or paste the files. I will review for idiom and structure and explain every correction. Expect comments on error handling, module layout, and clap usage; those are the habits worth forming now.

## Questions worth asking me while you build

- "Why does `?` not work in this function?" (return type)
- "What is the difference between `String` and `&str` and why does clap want one or the other?"
- "Why does clippy pedantic complain about X and should I fix it or allow it?"
