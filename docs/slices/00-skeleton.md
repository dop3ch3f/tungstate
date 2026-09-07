# Slice 0: Get a working Rust project

**Goal:** a `tungstate` command that runs, a test suite that passes, and a linter that keeps you honest. No real features. This is the workbench you will build everything else on.

**Time:** an afternoon, most of it reading.

## How this slice is different

From slice 1 onward I describe what to build and you write the code. Slice 0 is the exception. Almost all of it is project configuration, and nobody learns Rust by hand-typing a manifest file. So here I give you everything, and your job is to run it, see it work, and understand what each piece does. The "Understanding what you built" section at the end is the actual learning; do not skip it.

Every command below was run and verified. The outputs shown are real.

---

## Step 1: Open the project

```
cd ~/Documents/Codes/Personal/tungstate
zed .
```

You will use Zed for editing and the terminal for running things. Zed has a built-in terminal at ctrl-` if you prefer one window.

---

## Step 2: Pin the Rust version

Create `rust-toolchain.toml` in the project root:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

**What this does:** anyone who clones this repo, including CI and future you, gets the same compiler and the same tools automatically. Without it, a teammate on an older Rust hits errors you cannot reproduce.

Only the two tools CI needs are listed. Editor tooling goes in your own rustup setup, not in a file CI obeys, otherwise every CI run downloads about 60 MB it never uses. Run this once on your machine:

```
rustup component add rust-analyzer rust-src
```

---

## Step 3: Create the workspace root

Create `Cargo.toml` in the project root:

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
clap = { version = "4.6", features = ["derive"] }
anyhow = "1.0"        # unused until slice 1; declared here so versions live in one place
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
insta = "1.48"
proptest = "1.11"
assert_cmd = "2.2"
predicates = "3.1"

[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
```

**Note there is no `[package]` section.** This file is not a crate. It is a workspace: a container that holds several crates, shares one lockfile and one build directory between them, and declares dependency versions once so the crates cannot drift apart.

---

## Step 4: Create the two crates

```
cargo new --lib crates/tungstate-api
cargo new --bin crates/tungstate-cli
```

You should see:

```
    Creating library `tungstate-api` package
    Creating binary (application) `tungstate-cli` package
```

**What just happened, and it is smarter than you would expect.** Cargo noticed the workspace and wrote both manifests to inherit from it, so they already say `version.workspace = true` and `[lints] workspace = true`. You do not have to add the crates to the members list either, because `crates/*` is a glob that already matches them.

Two crates, because:

- `tungstate-api` is a **library**. It has `src/lib.rs` and produces no executable. It will hold the types the daemon and every client share.
- `tungstate-cli` is a **binary**. It has `src/main.rs` and produces the program you run.

Libraries hold logic and are easy to test. Binaries stay thin. That split is why the CLI, the TUI, and the web interface will later be able to share one definition of everything.

---

## Step 5: Fill in `tungstate-api`

Replace `crates/tungstate-api/Cargo.toml` with:

```toml
[package]
name = "tungstate-api"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true

[dependencies]
serde = { workspace = true }

[dev-dependencies]
proptest = { workspace = true }
serde_json = { workspace = true }

[lints]
workspace = true
```

`[dependencies]` ship in the final program. `[dev-dependencies]` exist only when running tests, so your test tooling never bloats the binary you distribute.

Replace `crates/tungstate-api/src/lib.rs` with:

```rust
//! Types shared by the tungstate daemon and every client (CLI, TUI, web).
//!
//! Everything here is a wire contract. Changing a type in this crate must keep
//! older clients working, so additions are safe and removals are not.

use serde::{Deserialize, Serialize};

/// Version of the tungstate binary, filled in by Cargo at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Identifier for a governed folder.
///
/// A newtype rather than a bare `String` so a folder id can never be passed
/// where a link id is expected. This costs nothing at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FolderId(pub String);

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn folder_id_round_trips_through_json(s in ".*") {
            let original = FolderId(s);
            let json = serde_json::to_string(&original).unwrap();
            let parsed: FolderId = serde_json::from_str(&json).unwrap();
            prop_assert_eq!(original, parsed);
        }
    }
}
```

---

## Step 6: Fill in `tungstate-cli`

Replace `crates/tungstate-cli/Cargo.toml` with:

```toml
[package]
name = "tungstate-cli"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true

[[bin]]
name = "tungstate"
path = "src/main.rs"

[dependencies]
tungstate-api = { path = "../tungstate-api" }
clap = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }

[dev-dependencies]
assert_cmd = { workspace = true }
predicates = { workspace = true }
insta = { workspace = true }

[lints]
workspace = true
```

The `[[bin]]` block is what makes the command `tungstate` rather than `tungstate-cli`. Without it the executable takes the package name.

Replace `crates/tungstate-cli/src/main.rs` with:

```rust
//! The `tungstate` command line interface.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "tungstate", version = tungstate_api::VERSION, about = "Keep folders in the shape you declared.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage governed folders.
    Folder {
        #[command(subcommand)]
        action: FolderAction,
    },
    /// Manage transfer links between folders.
    Link {
        #[command(subcommand)]
        action: LinkAction,
    },
}

#[derive(Subcommand)]
enum FolderAction {
    /// Start governing a folder.
    Add {
        /// Path to the folder to govern.
        path: String,
    },
}

#[derive(Subcommand)]
enum LinkAction {
    /// Create a transfer link from a source to a destination.
    Add {
        /// Where files come from.
        from: String,
        /// Where files go.
        to: String,
    },
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    // Every subcommand is a stub until its slice lands. Exit code 2 keeps
    // "not built yet" distinguishable from a real runtime failure (1).
    match cli.command {
        Command::Folder { action } => match action {
            FolderAction::Add { path } => {
                tracing::debug!(%path, "folder add requested");
            }
        },
        Command::Link { action } => match action {
            LinkAction::Add { from, to } => {
                tracing::debug!(%from, %to, "link add requested");
            }
        },
    }

    eprintln!("not implemented yet");
    std::process::ExitCode::from(2)
}
```

---

## Step 7: Write the tests

Create the folder and file `crates/tungstate-cli/tests/cli.rs`:

```rust
//! End-to-end tests that run the built `tungstate` binary.

use assert_cmd::Command;

fn tungstate() -> Command {
    Command::cargo_bin("tungstate").expect("binary `tungstate` should be built by cargo test")
}

#[test]
fn version_flag_prints_the_api_version() {
    tungstate()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicates::str::contains(tungstate_api::VERSION));
}

#[test]
fn folder_add_is_a_stub_for_now() {
    tungstate()
        .args(["folder", "add", "/tmp/example"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("not implemented yet"));
}

#[test]
fn help_output_is_stable() {
    let output = tungstate().arg("--help").output().expect("help should run");
    let help = String::from_utf8(output.stdout).expect("help output should be utf-8");
    insta::assert_snapshot!(help);
}
```

A file in `tests/` is an **integration test**. It compiles as a separate program that uses your crate the way a real user would. Unit tests, like the one in `lib.rs`, live inside the file they test and can see private items.

---

## Step 8: Build and run

```
cargo build
```

First build takes about half a minute because it compiles every dependency. Later builds are seconds.

```
cargo run --bin tungstate -- --version
```

Expect `tungstate 0.0.1`. The bare `--` separates arguments for cargo from arguments for your program.

Try these too:

```
./target/debug/tungstate folder add /tmp/x     # prints "not implemented yet", exit 2
./target/debug/tungstate --help                # the command surface
RUST_LOG=debug ./target/debug/tungstate folder add /tmp/x   # now the tracing line appears
```

That last one is worth pausing on. The log line was always in the code, but hidden until you asked for it. That is `tracing` doing its job, and it is how you will debug every later slice.

---

## Step 9: The test suite, including one deliberate surprise

```
cargo test
```

**The first run fails, and that is correct.** You will see a red block about `help_output_is_stable` and a snapshot summary. Snapshot testing means "record what the output looks like now, then shout if it ever changes." On the very first run there is nothing recorded yet, so insta writes what it saw to a `.snap.new` file and fails, asking you to approve it.

Read the diff. If the help text looks right, accept it:

```
INSTA_UPDATE=always cargo test
```

Then run `cargo test` again normally. You should now see:

```
test tests::folder_id_round_trips_through_json ... ok
test version_flag_prints_the_api_version ... ok
test folder_add_is_a_stub_for_now ... ok
test help_output_is_stable ... ok
```

From now on, if you ever change a command name or a help string, that test fails and shows you exactly what changed. That is the point: your command line interface cannot drift without you noticing.

Add this line to `.gitignore` so pending snapshots are never committed:

```
*.snap.new
```

---

## Step 10: The linter

```
cargo clippy --all-targets -- -D warnings
```

Silence means success. Clippy is not a style checker, it is a Rust-idiom teacher. We turned on `pedantic`, which is stricter than most projects use, deliberately: when clippy tells you there is a better way to write something, that is free senior-engineer advice at the moment you need it. Read every suggestion rather than reflexively silencing it.

```
cargo fmt --all
```

This rewrites your files to the community standard layout. There is no formatting debate in Rust, which is a mercy. Run it before every commit.

---

## Step 11: Continuous integration

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  check:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      # Components come from rust-toolchain.toml, so none are listed here.
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      # --locked fails the build on a stale lockfile instead of silently
      # resolving new dependency versions nobody reviewed.
      - run: cargo fmt --all --check
      - run: cargo clippy --all-targets --locked -- -D warnings
      - run: cargo test --locked
```

Windows is in the matrix from day one on purpose. Tungstate is a file tool, path handling differs on Windows, and those bugs are far cheaper to find now than in slice 6.

Commit and push, then watch the run on GitHub. Commit `Cargo.lock` too. For an application, as opposed to a library, the lockfile is part of the source: it is what makes a build today identical to a build in a year.

Do not create a `src/` directory at the repo root. The root is a workspace, not a crate, so cargo would ignore anything you put there.

---

## Understanding what you built

Come back to this after everything is green. These are the ideas you will use in every later slice.

**Crates and modules.** A crate is one compilation unit with one `Cargo.toml`. A module is a namespace inside a crate. We split into crates for compile boundaries and build parallelism, not for tidiness.

**`pub` means public.** Anything without it is private to its module. Rust defaults to private, so you expose deliberately.

**The newtype pattern.** `struct FolderId(pub String)` wraps a string in a distinct type. At runtime it is just a string with zero overhead. At compile time it is a different type, so passing a folder id where a link id belongs will not compile. You will see this pattern constantly in good Rust, and we will add more of them as the project grows.

**`derive` writes code for you.** `#[derive(Debug, Clone, Serialize)]` generates the implementations at compile time. `Debug` gives you `{:?}` printing, which is what `dbg!()` and log lines rely on. Getting `Debug` on your types early makes everything easier to inspect.

**Enums carry data.** `Command::Folder { action }` is not just a tag, it holds a value. Rust enums are closer to a tagged union than to an enum in most languages, and `match` forces you to handle every case. This is the feature you will lean on hardest in slice 3, where the transfer state machine is one enum.

**Exit codes are an interface.** Zero means success, one conventionally means failure, and we chose two for "not built yet." Scripts read these, so they are part of your contract just like the help text.

**Property tests.** The proptest in `lib.rs` does not check one example. It generates many random strings and asserts the round trip holds for all of them. That style matters enormously for tungstate, because in slice 6 we will assert things like "applying a plan then re-planning produces no further changes" across randomly generated folder trees.

---

## Acceptance criteria

- [ ] `cargo build` succeeds with no warnings
- [ ] `cargo run --bin tungstate -- --version` prints `tungstate 0.0.1`
- [ ] `tungstate folder add x` prints `not implemented yet` and exits 2
- [ ] `RUST_LOG=debug` reveals the tracing line
- [ ] `cargo test` passes all four tests
- [ ] `cargo clippy --all-targets -- -D warnings` is silent
- [ ] `cargo fmt --all --check` is silent
- [ ] CI is green on Linux, macOS, and Windows
- [ ] `*.snap.new` is in `.gitignore`
- [ ] No `unwrap()` in `src/`. Tests may use it freely.

---

## Deliberately break things

You learn more from errors than from success. Try each of these, read the message, then undo it.

1. Delete `#[derive(Debug)]` from `FolderId` and run `cargo test`. Read what the compiler says about `Debug`.
2. Change `path: String` to `path: u32` in `FolderAction::Add`, then run `tungstate folder add /tmp/x`.
3. Change the `about` text and run `cargo test`. Watch the snapshot test catch you.
4. Add `let x = 5;` to `main` without using it. See what clippy and the compiler say.

Rust error messages are unusually good. Reading them properly is a skill, and it is most of the learning curve.

---

## When you are done

Commit, push, and tell me. I will review and explain anything I would have done differently.

Ask me anything while you work. Good questions for this slice: why `&str` and not `String`, what `#[cfg(test)]` actually does, why `main` returns `ExitCode`, and what the `?` operator will do once we start using it in slice 1.
