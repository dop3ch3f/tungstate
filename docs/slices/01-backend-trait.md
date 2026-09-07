# Slice 1: The Backend trait and a local implementation

**Goal:** one interface that describes "a place files live", and one implementation of it for the local disk. Everything later in tungstate talks to storage only through this interface, so the transfer engine will work over SFTP without knowing SFTP exists.

**Runnable outcome:** `cargo test` proves you can create a directory, write a file, stat it, list it, read it back, and rename it, all through a `Box<dyn Backend>`, and that a malicious path cannot escape the root.

**This is your first real code.** I describe the shapes and the reasoning. You write it. Where I show code below it is to pin down an interface we both have to agree on, not to save you typing.

---

## Why this slice exists before anything else

Slice 3 builds the drain: copy a file to the NAS, verify it, delete the source. That code should not care whether the destination is a mounted volume or an FTP server. If it does, you write it twice.

So the drain will be written against `Backend`, not against `std::fs`. This slice defines that boundary. Get the shape right and slice 12 adds FTP by writing one new implementation and changing nothing else.

---

## Step 1: Create the crate

```
cargo new --lib crates/tungstate-backend
```

Then add to `[workspace.dependencies]` in the root `Cargo.toml`:

```toml
tempfile = "3.27"
```

`thiserror` is already declared there. In the new crate's `Cargo.toml` you need `thiserror` as a dependency and `tempfile` as a dev-dependency.

**Check `cargo new` did not leave a nested git repo.** It did that last time:

```
ls -a crates/tungstate-backend | grep -E '^\.git'
```

If anything shows up, delete `crates/tungstate-backend/.git` and `crates/tungstate-backend/.gitignore`. The workspace root owns version control.

---

## Step 2: The error type

Every fallible operation returns `Result<T, BackendError>`. Define it with `thiserror`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("path `{0}` escapes the backend root")]
    PathEscapesRoot(PathBuf),
    #[error("path `{0}` must be relative to the backend root")]
    PathNotRelative(PathBuf),
    #[error("io error at `{path}`")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, BackendError>;
```

**Why not just use `std::io::Error`?** Because `std::io::Error` does not tell you *which path* failed. When a drain of 4000 files reports "permission denied", you need to know which file. Wrapping the io error with its path is the difference between a usable log and a useless one.

**What `#[source]` does.** It links this error to the one that caused it, so a caller can walk the chain and print "io error at /x/y.mp4: caused by: permission denied". `thiserror` generates that plumbing.

**Why a crate-local `Result` alias.** So every signature reads `Result<Meta>` instead of `Result<Meta, BackendError>`. This is standard practice in Rust libraries. Note it shadows `std::result::Result` inside this crate, which is why the alias itself must spell out the full path.

---

## Step 3: The data types

```rust
pub struct Meta {
    pub len: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub modified: Option<SystemTime>,
}

pub struct Entry {
    pub path: PathBuf,
    pub meta: Meta,
}

pub struct Capabilities {
    pub atomic_rename: bool,
    pub hard_links: bool,
    pub case_sensitive: bool,
}
```

Derive `Debug` and `Clone` on all three, plus `PartialEq, Eq` so tests can compare them, and `Copy` on `Capabilities` since it is three bools.

**Why `modified` is an `Option`.** Not every filesystem or protocol reports a modification time. FTP servers are notoriously vague about it. `Option` forces every caller to decide what to do when it is missing, at compile time, instead of discovering it in production. This is Rust's central bargain: the type system makes you handle the sad path.

**`Path` versus `PathBuf`.** `Path` is a borrowed slice, like `&str`. `PathBuf` is owned, like `String`. Take `&Path` as a parameter, because that lets a caller pass either without copying. Store and return `PathBuf`, because the struct must own its data. Getting this pair right is most of what "ownership" means day to day.

---

## Step 4: The trait

```rust
pub trait Backend: Send + Sync {
    fn capabilities(&self) -> &Capabilities;
    fn stat(&self, path: &Path) -> Result<Meta>;
    fn read_dir(&self, path: &Path) -> Result<Vec<Entry>>;
    fn open_read(&self, path: &Path) -> Result<Box<dyn Read + Send>>;
    fn create_write(&self, path: &Path) -> Result<Box<dyn Write + Send>>;
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    fn remove_file(&self, path: &Path) -> Result<()>;
    fn create_dir_all(&self, path: &Path) -> Result<()>;
}
```

Four decisions here that you should be able to defend, because a reviewer will ask.

**All paths are relative to the backend's root.** Never absolute. The backend joins them to its root itself. This is what makes "tungstate can never touch anything outside the folder it governs" a property of the type system rather than a promise in a comment.

**`open_read` returns `Box<dyn Read + Send>`, not `impl Read`.** You will be tempted by `impl Read` because it is faster and prettier. It also makes the trait not object-safe, which means `Box<dyn Backend>` stops compiling. Since the planner must hold "some backend, decided at runtime", we need the box. This is the classic static-versus-dynamic dispatch trade, and here flexibility wins. The `Send` bound is there so slice 4 can hash a file on a worker thread.

**Returning a reader rather than bytes.** `open_read` hands back something you pull bytes from, so a 40 GB video never sits in memory. If this returned `Vec<u8>` the drain would die on the first large file.

**`read_dir` returns a `Vec` for one directory only, not a recursive walk.** One directory is bounded, so a `Vec` is honest. Recursive walking over a million files needs streaming, and that is slice 6's problem. Do not build it now.

---

## Step 5: The path resolver, which is the important part

This is the security boundary of the whole product. Everything else in this slice is plumbing.

Write a private function that turns a caller's relative path into a real filesystem path, or refuses:

```rust
fn resolve(root: &Path, path: &Path) -> Result<PathBuf>
```

Rules: reject absolute paths, and reject any path containing a `..` component. Accept normal components and `.`. Then join to root.

**Why not `canonicalize`?** Because it requires the file to already exist, and you need to resolve paths for files you are about to create. Rejecting `..` up front is simpler and works on paths that do not exist yet.

Walk `path.components()` and match on `Component`. The variants you accept are `Normal` and `CurDir`. Everything else, which is `ParentDir`, `RootDir`, and `Prefix`, is a rejection. Writing this as a `match` rather than a chain of `if`s means the compiler tells you if a new variant ever appears.

**This function needs adversarial tests.** At minimum: `../etc/passwd`, `a/../../b`, `/etc/passwd`, and a legitimate `a/b.txt` that must pass. Write those tests before the implementation if you like; they are the specification.

---

## Step 6: `LocalBackend`, with capabilities discovered by experiment

```rust
pub struct LocalBackend {
    root: PathBuf,
    capabilities: Capabilities,
}

impl LocalBackend {
    pub fn open(root: PathBuf) -> Result<Self> { ... }
}
```

Then implement every `Backend` method using `std::fs`. Most are two lines: resolve the path, call the `std::fs` function, map the io error into `BackendError::Io` with the path attached.

The interesting part is `capabilities`. **Do not detect them by checking the operating system.** "macOS means case-insensitive" is wrong the moment someone formats a case-sensitive APFS volume, and "Linux means hardlinks" is wrong on a FAT-formatted USB stick. The filesystem is the thing that matters, not the OS, and the only reliable way to know what a filesystem does is to try it.

So at `open` time, probe:

- **Hard links:** write a temp file under the root, try `std::fs::hard_link` to a second name, record whether it worked, delete both.
- **Case sensitivity:** write a file with a lowercase name, then ask whether the uppercase name exists. If it does, the filesystem is case-insensitive.

Clean up your probe files whether the probe succeeded or failed. Use a distinctive prefix like `.tungstate-probe-` so a leftover is obviously ours.

**Leave `atomic_rename` as `true`** for local filesystems. It becomes interesting in slice 12, where some FTP servers cannot do it.

For reference, on your Mac the probe reports `hard_links: true, case_sensitive: false`, because APFS is case-insensitive by default. Linux CI will report `case_sensitive: true`. That difference is real and it is exactly why you are probing.

---

## Step 7: Tests

Use `tempfile::tempdir()` for every test so they are parallel-safe. Each test gets its own directory and they cannot collide. Never use a fixed path under `/tmp`.

Write at least these:

1. **Object safety.** A test that puts a `LocalBackend` into a `Box<dyn Backend>` and uses it. If the trait ever stops being object-safe, this fails to compile, which is the earliest possible warning.
2. **Round trip.** Through the trait object: `create_dir_all`, `create_write` and write bytes, `stat` and check the length, `read_dir` and check the count, `open_read` and compare the contents.
3. **Traversal rejection.** The adversarial paths from step 5.
4. **Error content.** Stat a file that does not exist and assert you get `BackendError::Io` whose `path` is the one you asked for. This is the test that proves your error type is actually useful.

One subtlety in the round-trip test: `create_write` returns a boxed writer that holds an open file handle. Drop it, by ending its scope with a block or calling `drop`, before you `stat` the file. On Windows an open handle can block other operations, and this is the kind of thing CI will catch if you get it wrong.

---

## Two things that will bite you

**Clippy will demand `# Errors` documentation.** The pedantic lint set includes `missing_errors_doc`, which fires on every public function returning `Result`. Slice 0 had none, so you have not met it. Slice 1 has eight, so you will see eight errors that look like this:

```
error: docs for function returning `Result` missing `# Errors` section
```

The fix is a doc comment section on each one:

```rust
/// Read metadata for `path`.
///
/// # Errors
/// Returns [`BackendError::PathEscapesRoot`] if `path` tries to leave the root,
/// or [`BackendError::Io`] if the file cannot be read.
fn stat(&self, path: &Path) -> Result<Meta>;
```

Do not silence this lint. Writing these sections forces you to enumerate how each method fails, which for a tool that moves people's files is the most valuable thinking in the slice.

**Do not write a test that asserts case sensitivity.** It would pass on your Mac and fail on Linux CI, or the reverse. Test that the probe *returns something*, and that the round trip works, not what the answer is. Remember the Windows failure in slice 0: the same class of mistake, which is a test encoding a local assumption as a universal truth.

---

## Acceptance criteria

- [ ] `crates/tungstate-backend` exists with no nested `.git`
- [ ] `cargo build --locked` is clean
- [ ] `cargo test --locked` passes, including a traversal-rejection test and a test through `Box<dyn Backend>`
- [ ] `cargo clippy --all-targets --locked -- -D warnings` is silent, with real `# Errors` docs rather than an `allow`
- [ ] `cargo fmt --all --check` is silent
- [ ] CI green on all three platforms
- [ ] No test depends on case sensitivity, hardlink support, or a fixed temp path
- [ ] No `unwrap()` in `src/`, freely in tests

---

## Deliberately break things

1. Change `open_read` to return `impl Read` instead of `Box<dyn Read + Send>`. Watch the object-safety test fail to compile, and read that error carefully. It is one of the more confusing messages in Rust and worth meeting on purpose.
2. Remove the `..` check from `resolve` and run the traversal test. Confirm the test actually catches it, because a security test that passes for the wrong reason is worse than none.
3. Take `path: PathBuf` instead of `&Path` in `stat`, then try calling it twice with the same variable. Read what the compiler says about a moved value. That message is ownership in one paragraph.

---

## What I want to see in review

Push a branch and I will look at: whether `resolve` is airtight, whether your `# Errors` docs describe real failure modes rather than restating the type, whether the io errors carry enough context to debug a 4000-file drain, and whether any test would behave differently on Linux.

Ask questions freely. Good ones for this slice: when to use `&Path` versus `PathBuf`, why `Box<dyn Trait>` needs the `dyn`, what `Send` and `Sync` actually promise, and how `?` interacts with `map_err`.
