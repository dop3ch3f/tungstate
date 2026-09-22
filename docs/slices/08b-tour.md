# Slice 8b tour: duplicates

`tungstate dedupe` finds files that are the same file, says which copy is
worth keeping, and sets the extras aside or sends them to the trash. This is
the walk through what shipped, why it is shaped that way, and the two bugs it
found that were older than it.

Brief: [08b-dedup.md](08b-dedup.md). Decisions: DESIGN §5, amended here.

---

## 1. The pass that does no reading

`crates/tungstate-core/src/dupes.rs` is the whole of the thinking, and it
cannot open a file. It takes a `Snapshot` (pure data), and anything it needs
read it asks for:

```rust
pub trait Digest {
    fn partial(&mut self, path: &str) -> Result<String, String>;
    fn whole(&mut self, path: &str) -> Result<String, String>;
}
```

**A trait is Rust's word for "something that can do these things"**, and
passing `&mut dyn Digest` means "anything that can, decided at runtime". The
planner already works this way, and the payoff is the same: the tests hand it
a `Table` that maps paths to strings, so twelve awkward folders are twelve
short functions rather than twelve temporary directories.

Two methods rather than one because the second is the expensive one. The pass
asks `partial` of everything whose size matched something else, and `whole`
only of what survived that. `a_file_alone_at_its_size_is_never_read` asserts
the tiers actually save the work they exist to save: three files of different
sizes, zero reads.

### Grouping, in three `BTreeMap`s

```rust
let mut by_size: BTreeMap<u64, Vec<&Attributes>> = BTreeMap::new();
```

`BTreeMap` rather than `HashMap` for the reason the planner has in a comment:
a hash map's iteration order is randomised per process, and "the same folder
gives the same answer" is an acceptance criterion here too. The `&Attributes`
inside are **borrows** of the snapshot's entries: nothing is copied, and the
compiler is what guarantees the snapshot outlives the borrows.

## 2. Which copy stays, and why the reason is part of the answer

`choose()` implements DESIGN §5's tie-break: pinned, then already where the
rules would put it, then the oldest, then the plainest path. It returns the
path *and* a `Kept` saying which rule decided, and the report prints it:

```
big.bin (293.0 KB, kept: it is the oldest)
```

Two of those rules are deliberately quiet when they do not separate anything.
If every copy is where the rules would put it, "kept because it is in its
place" is true of the others too; if every copy shares one mtime, "kept
because it is the oldest" is a lie about the reason. Both fall through to the
next rule instead. The second case is not rare: **FTP rounds mtimes to the
minute** (DESIGN §9), so equal ages are the ordinary case there.

`birthtime` is what DESIGN names and almost no backend reports, so mtime
stands in. A copy usually carries the original's mtime, which is what makes
that a stand-in rather than a guess.

## 3. Folders copied whole

A directory is a copy of another when its files match by content **and by
layout**: the fingerprint is the sorted list of `relative path + hash` inside
it. Content alone would call two directories holding the same five videos
under different names "the same folder", and hide a real difference behind a
bulk action.

Files inside a copied folder are not offered again as individual duplicates.
That is the `covered` set in `find()`: one decision, not four hundred.

## 4. Two new operations, and how the compiler found every place they mattered

`Op::Trash { path, of }` and `Parked::Duplicate { of }` are new variants on
existing enums. Adding a variant to a Rust enum makes every `match` on it stop
compiling until it is handled, and that is the point: the compiler listed the
six places that had to be told about a trash, including two in the window's
seam that nobody would have thought to grep for.

`Op::Trash` is the first operation in this project that removes content, and
the module comment that used to say *no operation here removes content* now
says exactly where the exception is and who may emit one. `Op::SetAside` from
the brief turned out not to be needed: setting a copy aside is
`Op::Quarantine` with a new reason, and reusing it meant the executor, the
journal and `undo` all already knew what to do.

## 5. Trashing needs a real path, so the trait grew a question

`trash::delete` drives the desktop's Finder and knows nothing about an FTP
server. Rather than let the CLI decide, `Backend` gained one method with a
default:

```rust
fn on_this_machine(&self, path: &Path) -> Result<Option<PathBuf>> {
    Ok(None)
}
```

**A default method** means every existing backend and every test double keeps
compiling and answers "no". `LocalBackend` overrides it. The executor refuses
a trash it cannot place, in the same shape the drain already refuses a remote
trash.

## 6. The hash cache, and the thing that makes a cache safe

`crates/tungstate-execute/src/digest.rs` reads through a backend and writes
every answer into the journal's new `hashes` table. The key is `(root, path)`;
the **check** is size and mtime. A file edited in place keeps its name, so
without that check a remembered digest would be a wrong answer about
somebody's files rather than a fast one.

Measured on a demo folder: first pass 16 reads, second pass 0 reads and 16
recalled. Edit one file in place without changing its size, and exactly one
file is read again.

Slice 6 dropped the index because a cache with no reader earns nothing. This
one has a reader on the day it lands, which is the difference.

## 7. The conversation, which is the CLI's job

`dedupe` with no flags reports and changes nothing. `--apply` needs to know
what happens to the extras, and **nothing here picks for you**: `--extras
set-aside|trash`, or it asks, once, and remembers the answer in the new
`settings` table. `--yes` means "do not stop to ask", so it takes the only
answer that cannot lose anything: set aside.

The window's theme switch will use the same table, which is why it is a table
rather than a column.

## 8. Two bugs older than this slice

**No tidy that set a file aside could ever be undone.** Undo inverts the
journal's operations and asks slice 6's paper model whether the result can be
carried out. A file under `.tungstate-quarantine` is in no snapshot, because
the walk is told never to enter it (otherwise a parked file is classified
again and routed straight back out, and planning never converges). So every
reverse move out of there was "there is nothing there to move", and every
reverse `rmdir` was "there is no such directory". `replay` now takes the
journal's word for paths inside the reserved area, with the reason written
where the next person will hit it. `setting_duplicates_aside_can_be_undone`
is the regression test.

This has been true since slice 7. It needed a plan that parks something *and*
an undo of that plan, and `on_conflict = "quarantine"` never had one.

**"Moved 9 files" was about to happen again.** The first report printed
`applied.done`, which counts operations: five files set aside came out as
eleven, because the plan also removes the directories it empties. `Found` now
has `extra_files()`, and the brief's property 3 keeps its teeth.

## 9. Two names for one file, and one name for two files

**Hard links.** Two names for one file are not two copies: setting one aside
reclaims nothing, and reporting it as a duplicate is reporting a saving that
does not exist. The identity comes from the stat the survey already does, so
it costs no extra reads: `Meta.identity` is the volume plus the file number on
a local disk, `None` everywhere else, and only carried when the file has more
than one name. Paths sharing one identity collapse to the plainest of them
before grouping, and the rest are reported separately:

```
two names for one file
  clip.mp4  =  backup/clip-link.mp4 (195.3 KB)
  These are hard links, not copies: dealing with one frees nothing.
```

`None` means "no idea", never "different files", which is what keeps a remote
backend from claiming two copies are one.

**A name two files both want.** On a case-insensitive volume, the default on
macOS and Windows, `Clip.mp4` and `clip.mp4` are one name. Two files sent to
one set-aside name is a file lost at the moment of the move, so the plan
numbers the second: `clip-2.mp4`, the way the planner numbers a name two files
both asked for. The volume's own rule is `Snapshot::key`, which already
existed and this pass simply had not been consulting.

## 10. What is checked

Twenty-four new tests. The one that matters most is a property test:

```rust
proptest! {
    fn every_group_keeps_one_copy(contents in ...) { ... }
}
```

**`proptest` generates hundreds of folders** rather than the one you thought
of, and checks the same two claims each time: no path appears in two groups,
and after planning, exactly one copy of each distinct content is still in
place. That is the safety property the whole slice rests on, and it is now
impossible to break quietly.

The rest, briefly: identical content under different names is one group; a
file alone at its size is never read; same size and different ends is never
read in full; the oldest copy stays; one shared mtime falls through to the
path; a pin overrides; two pins in one group is refused rather than resolved;
a folder copied whole is one decision; `--files-only`; a copy already set
aside is not a duplicate of anything; empty files are not copies of each
other; the trash is only ever planned when asked for; a digest is remembered
and forgotten when size or mtime changes; a partial digest survives a whole
one being added; an answer is remembered; an irreversible plan is not offered
by `undo --last` and is refused by name by `undo --plan`.

`cargo fmt`, `clippy -D warnings`, 488 tests, CI green on Linux, macOS and
Windows.

## 11. What is not in this slice

- **The window.** Slice 8c: its own Duplicates section, groups with every copy
  shown, per-group overrides, and pick-by-rule. `--json` already emits what it
  needs, including a stable group id so a selection survives a rescan.
- **Near-duplicates.** Slice 8d.
- **Hard links and reflinks.** 15+, and they change what a file *is*.
- **Cross-folder duplicates.** One root at a time.
- **Remote trash.** `.tungstate-trash/` in DESIGN §4 is unbuilt; over a
  connection, set aside is the only action.

## What is still not verified

- **Only run on local folders.** The pass goes through `Backend`, so a
  connection should work, but full hashing over FTP has not been tried and
  would read every byte across the network. The brief's "warn before hashing a
  lot over a network" is **not implemented**: it needs a size threshold nobody
  has picked yet, and `LocalBackend` cannot currently tell a mounted NAS from
  the boot disk, which is the other half of the question.
- **Hard links on Windows are untested.** The identity there is the volume
  serial and the file index, which needs a real NTFS hard link to prove, and
  CI has never made one.
- **Scale.** The largest folder tried was a few hundred files. A drive with
  100,000 files will spend its time in `stat` and in SQLite, neither of which
  is measured.
