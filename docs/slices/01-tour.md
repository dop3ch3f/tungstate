# Slice 1 tour: the Backend trait and LocalBackend

Read this next to the diff. It walks the shipped code top to bottom and explains each Rust idea where it actually appears, rather than in the abstract.

Two files: `crates/tungstate-backend/src/lib.rs` is the interface, `src/local.rs` is the implementation for local disks. That split is the point of the slice. Slice 3's drain will be written against the interface, so it works over a mounted volume today and over FTP in slice 4b without changing a line.

---

## Part 1: The error type

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
```

**Rust has no exceptions.** A function that can fail returns `Result<T, E>`, which is an enum with two variants: `Ok(T)` and `Err(E)`. Nothing is thrown, nothing unwinds past you, and the type signature tells you what can go wrong. This is the single biggest day-to-day difference from most languages you know.

**`enum` here is not what `enum` means elsewhere.** In Rust an enum variant can carry data, and each variant can carry a different shape. `PathEscapesRoot` carries one `PathBuf`; `Io` carries a named struct with two fields. This is a tagged union, closer to a sealed class hierarchy than to a list of constants. You will see this again in slice 3, where the whole transfer state machine is one enum.

**`#[derive(...)]` writes code at compile time.** `thiserror::Error` generates the boilerplate that makes this type behave like a real error: the `Display` implementation comes from those `#[error("...")]` strings, and `#[source]` wires up the chain so a caller can walk from our error down to the operating system's. Without `thiserror` you would hand-write about forty lines of trait implementations.

**Why the `Io` variant carries a path.** `std::io::Error` tells you *what* went wrong but never *where*. Halfway through draining four thousand videos, "permission denied" with no filename is unactionable. This is the kind of decision that costs one field now and saves an hour of debugging later.

```rust
pub type Result<T> = std::result::Result<T, BackendError>;
```

A type alias so every signature reads `Result<Meta>` instead of `Result<Meta, BackendError>`. Standard practice in Rust libraries. Note it has to spell out `std::result::Result` on the right, because by the time the compiler reads that line the name `Result` already refers to the alias being defined.

---

## Part 2: The data types, and the ownership idea

```rust
pub struct Entry {
    pub path: PathBuf,
    pub meta: Meta,
}
```

**`Path` versus `PathBuf` is the whole ownership model in one pair.** `PathBuf` owns its memory, like `String`. `Path` is a borrowed view into someone else's memory, like `&str`. The rule you will apply constantly:

- **Take `&Path` as a parameter.** The caller keeps ownership and nothing is copied.
- **Store and return `PathBuf`.** A struct must own what it holds, because the thing it borrowed from might vanish.

That is why `stat(&self, path: &Path)` borrows but `Entry { path: PathBuf }` owns. Get this pair right and most borrow-checker fights never start.

```rust
pub modified: Option<SystemTime>,
```

**`Option<T>` is Rust's answer to null**, and it is a genuine improvement rather than a rename. There is no null. If a value might be absent, the type says so, and the compiler will not let you use it without deciding what happens when it is missing. Not every filesystem reports a modification time and FTP is especially vague, so this one is honestly optional. Slice 3 will have to decide what a missing timestamp means, and the type is what forces that conversation.

---

## Part 3: The trait, and the decision that shapes everything

```rust
pub trait Backend: Send + Sync {
    fn capabilities(&self) -> Capabilities;
    fn stat(&self, path: &Path) -> Result<Meta>;
    fn open_read(&self, path: &Path) -> Result<Box<dyn Read + Send>>;
    ...
}
```

**A trait is a set of behaviours a type can promise to provide.** Closest thing to an interface in other languages, but more capable, since traits can carry default implementations and be added to types you did not write.

**`: Send + Sync` are promises about threads.** `Send` means a value can be moved to another thread. `Sync` means `&T` can be shared across threads. Slice 4 will hash files on worker threads, so requiring both here means the compiler proves the design works long before the threads exist. If you ever add a field that is not thread-safe, this line is what fails the build.

### The `Box<dyn Read + Send>` decision

This is the one to understand properly, because it looks like a wart and is not.

`open_read` could have returned `impl Read`, which is faster and reads better. It would also have broken the entire design. Here is why.

Rust has two ways to be generic:

- **Static dispatch.** `impl Read` means "some concrete type decided at compile time." The compiler generates a specialised copy of the function for each type. No runtime cost, but the type must be known when compiling.
- **Dynamic dispatch.** `Box<dyn Read>` means "some type decided at runtime, reached through a pointer." One pointer hop per call, and the type can vary.

Tungstate needs the second. The drain holds "whatever backend the user configured", which is a `Box<dyn Backend>`, decided when the config is read. But a trait can only be used as `dyn Trait` if it is **object safe**, and returning `impl Trait` from a method destroys that property. Write `impl Read` and `Box<dyn Backend>` stops compiling, with an error message that is not obvious the first time you meet it.

So the box is the price of runtime choice, and runtime choice is the product. One pointer hop per read call, against a function that then streams gigabytes, is not measurable.

**Why a reader at all, rather than `Vec<u8>`?** Because `open_read` returning bytes would load a 40 GB video into memory. A reader hands back something you pull bytes from in chunks, which is what makes the drain possible on a laptop that is already full.

**`read_dir` returns `Vec<Entry>`, and that is deliberate.** One directory is bounded, so collecting it is honest. A recursive walk over a million files is not, which is why recursion is slice 6's problem and needs streaming instead.

---

## Part 4: `resolve`, the part that actually matters

```rust
fn resolve(root: &Path, path: &Path) -> Result<PathBuf> {
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => {
                return Err(BackendError::PathNotRelative(path.to_path_buf()));
            }
            Component::ParentDir => {
                return Err(BackendError::PathEscapesRoot(path.to_path_buf()));
            }
        }
    }
    Ok(root.join(path))
}
```

Everything else in this crate is plumbing. This is half the security boundary of the whole product; the other half is `guarded()` below.

Every `Backend` method funnels through here, so a path that escapes the root cannot reach `std::fs`. That is what turns "tungstate can never touch anything outside the folder it governs" from a promise in a comment into a property of the code. A tool that deletes source files after copying them has to be airtight here.

**Why `match` instead of checking for `..`.** A `match` over an enum must handle every variant or the code does not compile. If a future Rust release adds a `Component` variant, this becomes a build error rather than a silently open door. Writing `if component == ParentDir` would have compiled fine and quietly let the new variant through. Reach for exhaustive `match` whenever missing a case is dangerous.

**That paid off within the hour, and it is the best thing in this slice.** The first version also had an `is_absolute()` check at the top. Windows CI failed on `/etc/passwd`. The reason is genuinely surprising:

On Windows, a path is only absolute if it has a drive prefix like `C:\`. A bare leading `/` is *root-relative*, so `Path::new("/etc/passwd").is_absolute()` returns **false** there. But `Path::join` does not care about `is_absolute()`. Its documented behaviour is that a path with a root but no prefix replaces everything except the prefix. So on Windows:

```
Path::new("C:\\governed-folder").join("/etc/passwd")  ==  "C:\\etc\\passwd"
```

The root is silently discarded. Had the code trusted `is_absolute()` alone, a caller passing `/etc/passwd` would have escaped the governed folder entirely on Windows, and passed every test on macOS and Linux. That is a real vulnerability, in the exact function whose job is to prevent it.

The exhaustive `match` caught it, because `RootDir` was already a component the code had to handle. The only thing actually wrong was which error it returned. The fix removed the `is_absolute()` check entirely, since inspecting components covers every platform with one rule, and split the variants so a rooted path reports `PathNotRelative` and a climbing path reports `PathEscapesRoot`.

Two lessons worth carrying: cross-platform assumptions about paths are usually wrong, and a test that asserts the *property* rather than the error message would have been clearer from the start. There is now a test doing exactly that, checking that no accepted path resolves outside the root, whatever the error type says.

**Why not `canonicalize`.** The obvious approach is to resolve the real path and check it starts with the root. It fails for our purposes, because `canonicalize` requires the file to already exist, and half our calls are for files about to be created. Component inspection works on paths that do not exist yet.

### The second way out, which `resolve` cannot see

An earlier version of this tour claimed that calling `symlink_metadata` in `stat` closed the symlink escape. That was wrong, and writing a test for the claim is what proved it.

`resolve` inspects path *text*. A symlink's name tells you nothing about where it points. So `downloads/holiday.mp4` is a perfectly innocent-looking path that `resolve` happily accepts, and if `holiday.mp4` is a link to `/etc/shadow`, then `open_read` hands you the contents of `/etc/shadow`. The path never left the root; the bytes did.

`create_write` was worse. A link named `escape` pointing at another directory meant `create_write("escape/planted.txt")` would write a file outside the governed folder, or overwrite one already there.

The fix is `guarded()`, which every method now calls instead of `resolve` directly. It walks the path one component at a time, `lstat`-ing each, and refuses if any of them is a symlink:

```rust
fn guarded(&self, path: &Path, tail: Tail) -> Result<PathBuf> {
    let full = resolve(&self.root, path)?;
    ...
}
```

**The `Tail` enum encodes a real distinction**, not fussiness. Some syscalls follow a link and some operate on the link itself:

- `open_read`, `create_write` and `read_dir` follow it, so the final component must not be a link. `Tail::MustNotBeLink`.
- `rename`, `remove_file`, `remove_dir` and `symlink_metadata` act on the link itself. Forbidding a link there would make links impossible to inspect or clean up, and the drain needs both. `Tail::MayBeLink`. Their parent directories are still checked.

Using an enum rather than a `bool` parameter is deliberate. `self.guarded(path, true)` at a call site tells the reader nothing; `Tail::MustNotBeLink` says exactly what is being asked for. Reach for a two-variant enum whenever a boolean parameter would be unreadable at the call site.

**Two honest limitations, both in the code's doc comment.** This costs one `lstat` per path component, which against a transfer that then streams gigabytes is not measurable. And it is not proof against an attacker who swaps a directory for a symlink in the window between the check and the open. Closing that needs `openat`-style per-component opens, which need `unsafe`, which the workspace lints forbid. It is a real limitation and it is written down rather than glossed over.

---

## Part 5: `io_at`, and closures that own their data

```rust
fn io_at(path: &Path) -> impl FnOnce(std::io::Error) -> BackendError {
    let path = path.to_path_buf();
    |source| BackendError::Io { path, source }
}
```

Used as `std::fs::rename(&a, &b).map_err(io_at(&a))?`, which turns nine repetitive lines into one per method.

**Read the two lines carefully, because the ordering is the lesson.** The function receives a borrowed `&Path`, then immediately calls `to_path_buf()` to make an owned copy. The closure captures that owned copy. It has to: the closure outlives this function call, and a borrow cannot outlive the thing it points at. Try to capture the `&Path` directly and the borrow checker refuses, correctly.

**`impl FnOnce(...) -> ...` is a return type meaning "a closure".** Closures each have their own anonymous type that you cannot name, so `impl Trait` is how you return one. `FnOnce` specifically means the closure may consume what it captured, which this one does when it moves `path` into the error.

**`?` is the early-return operator.** `expr?` unwraps an `Ok` and returns the `Err` from the enclosing function on failure. It only works in a function that returns `Result`, which is the usual reason it fails to compile.

---

## Part 6: Capability probing, and interior mutability

```rust
pub struct LocalBackend {
    root: PathBuf,
    capabilities: OnceLock<Capabilities>,
}

fn capabilities(&self) -> Capabilities {
    *self.capabilities.get_or_init(|| probe(&self.root))
}
```

**Why probe at all, rather than check the operating system?** Because the operating system is the wrong question. "macOS means case-insensitive" is wrong on a case-sensitive APFS volume, and "Linux means hard links" is wrong on a FAT-formatted USB stick. The filesystem decides, not the OS, and the only reliable way to know what a filesystem does is to try it.

Your Mac reports `hard_links: true, case_sensitive: false`. Linux CI reports `case_sensitive: true`. Both are correct, which is exactly why nothing is hardcoded and why no test asserts a particular answer.

**`OnceLock` is the interesting type here.** Look at the signature: `capabilities(&self)` takes an *immutable* reference, yet it writes a cached value. Normally Rust forbids mutating through `&self`. `OnceLock` is one of a small family of types offering **interior mutability**: they permit exactly one controlled mutation and handle the thread-safety themselves. `get_or_init` runs the closure the first time and returns the stored value forever after, and if two threads race, one wins and the other waits.

The alternative was `Mutex<Option<Capabilities>>`, which would work but locks on every read forever, to protect a value that changes once. `OnceLock` says what we mean.

**Why lazy rather than probing when the backend opens.** Probing writes files. Opening a backend to read from it should not scribble in the user's directory, and when slice 12 adds FTP, probing over the network on every open would be slow. Deferring it means the cost is paid only if someone actually asks.

**The unique filenames are a real bug fix, not decoration:**

```rust
static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);

fn probe_name(kind: &str) -> String {
    let seq = PROBE_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{PROBE_PREFIX}-{kind}-{}-{seq}", std::process::id())
}
```

The first version used fixed names. Two backends rooted at the same directory probing at once would then collide: one deletes the other's probe file mid-check, and the loser reports "hard links unsupported" when they are perfectly supported. Process id plus an atomic counter makes that impossible. `Ordering::Relaxed` is sufficient because we need uniqueness, not ordering between threads.

**Probe failure returns `false`, not an error.** Pointing a backend at a read-only or missing directory should not blow up a caller who only wanted to read. "Cannot hard-link here" is a true answer about the filesystem, not a malfunction. There is a test for exactly this.

---

## Part 7: The tests, and why they look like that

Eight tests, and three of them exist for reasons worth naming.

**`round_trips_a_file_through_the_trait_object`** puts a `LocalBackend` inside a `Box<dyn Backend>` and drives it from there. If the trait ever loses object safety, this stops compiling. That is the earliest possible warning, and it is why the test uses the box rather than the concrete type.

Note this block:

```rust
{
    let mut writer = backend.create_write(Path::new("sub/video.mp4")).unwrap();
    writer.write_all(b"hello").unwrap();
}
```

The braces are load-bearing. They end the writer's scope, which drops it, which closes the file handle. Windows refuses operations on a file that still has an open handle, so without the scope this test would pass on your Mac and fail in CI. That is precisely the bug shape that bit slice 0.

**`probes_the_filesystem_without_leaving_anything_behind`** checks that the directory is empty afterwards. It deliberately does *not* assert what the capabilities are, because the honest answer differs between your Mac and Linux CI. A test that encodes a local assumption as a universal truth is how slice 0's Windows failure happened.

**Every test calls `tempfile::tempdir()`.** Each gets its own directory, so the suite is safe to run in parallel and no test can see another's files. Never a fixed path under `/tmp`.

**One test is not a test but 256 of them.** `no_generated_path_ever_resolves_outside_the_root` is a *property test*: `proptest` generates random path segments, deliberately seeded with `..`, `.` and empty strings, assembles them into paths, and asserts the invariant holds for every one. When it finds a failure it shrinks the input to the smallest case that still breaks, which usually hands you the bug directly.

This matters more here than anywhere else in the codebase. A hand-written list of adversarial paths only contains the attacks you already thought of, and the Windows `is_absolute` bug is proof that the ones you did not think of are the ones that bite. Slice 6 leans on this style much harder, to assert that applying a plan and then re-planning produces no further changes, over randomly generated folder trees.

---

## Things worth knowing that are not in the code

**`--locked` refused the build when dependencies changed**, which is the flag doing its job. It exists so CI fails on a stale lockfile rather than silently resolving new dependency versions nobody reviewed. When you legitimately add a dependency, run `cargo build` once without it to update `Cargo.lock`, then commit the lockfile.

**The slice 1 brief claimed `thiserror` was already a workspace dependency. It was not** — slice 0 never added it. Found at build time and added. Mentioned because the brief is now wrong on that detail, and briefs are written before the code exists.

---

## What to look at in review

- Is `resolve` airtight? It is the security boundary; adversarial paths belong in the test list if you can think of one I missed.
- Do the `# Errors` doc sections describe real failure modes, or just restate the return type?
- Would any test behave differently on Linux or Windows than on your Mac?
- Is the trait's method set right? Adding a method later forces every implementation to change, so `remove_dir` is in now because slice 3 prunes emptied source directories.
