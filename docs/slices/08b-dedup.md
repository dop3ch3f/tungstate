# Slice 8b: duplicates

**Goal:** find files whose contents are identical, say which copy is the one
worth keeping, and set the others aside so the space comes back and nothing is
lost.

**Runnable outcome:** `tungstate dedupe ~/Movies` reports what it found;
`tungstate dedupe ~/Movies --apply` sets the extra copies aside; `tungstate
undo --last 1` puts them back.

This is the slice DESIGN §5 has been waiting for, and the one slices 9b–9e
need: folder sync reuses the set-aside area this brings.

---

## What is already here

- **Hashing.** `tungstate-attrs` streams a file through BLAKE3 a megabyte at a
  time, and `Attributes.hash` is filled at tier `whole`. Nothing yet asks for
  a hash unless a policy needs one.
- **Cost tiers.** `Tier::{Stat, Head, Meta, Whole}`, and a walk asks for the
  cheapest tier the work needs. Duplicate detection is the first thing in the
  project that wants `Whole` on purpose.
- **A plan, a preview and an undo.** `tungstate-core::plan` builds an op list,
  `tungstate-execute` applies it and journals it, and `undo` reverses a plan by
  id. The window already draws a preview from one.
- **A set-aside area.** The drain parks files under `.tungstate-quarantine`,
  and `Op::Quarantine` moves a file there inside a governed folder.
- **The invariant.** No op a plan can express removes content
  (DESIGN §5, amended in slice 6). This slice must keep that true.

## What is missing

- Nothing hashes files in bulk, and nothing remembers a hash it computed.
- Nothing groups files by content.
- Nothing chooses between copies.
- There is no `dedupe` command.

---

## Decisions to take before writing code

### 1. What happens to the extra copies

DESIGN §5 says the default is `trash`: move them to the operating system's
trash. **This brief proposes `set aside` instead**, into the folder's own
`.tungstate-quarantine`, with `trash` as a later addition.

Why: set-aside is reversible by the machinery that already exists. `undo`
moves a file back from quarantine today, and it is one backend rename, which
works the same on a local disk, a mounted NAS and over FTP. The OS trash is
none of those things: it is local-only, it is a different API on each
platform, and a file in it is outside anything the journal can put back. The
project's own invariant — nothing a plan can express removes content — stays
true for free.

The word on screen is **"set aside"**, the same word the drain already uses
for the same physical thing.

### 2. Where it runs

A separate pass, not part of `tidy`. A tidy answers "is every file where the
rules say", a dedupe answers "is this file the only copy". Merging them would
mean one preview with two unrelated reasons in it, and a tidy that can no
longer promise it only moves things.

### 3. What it looks at, and how it pays for it

Three tiers, in order, exactly as DESIGN §5 describes:

1. **Size.** A directory listing already has it. Only size-equal files can be
   duplicates, and most sizes are unique.
2. **Partial hash.** First and last 64 KiB of each candidate. Kills the false
   pairs cheaply, and is one read of 128 KiB rather than the whole file.
3. **Full BLAKE3**, only for files that survive both.

A hash is remembered in the journal, keyed by the path, its size and its
mtime, and forgotten the moment any of the three changes. Schema v9, one new
table, added to `storage::TABLES` (which is hand-maintained, and a table
missing from it silently misses export).

### 4. Which copy is canonical

DESIGN §5's tie-break, in order: already at the path the policy would give it
> older mtime > shorter path > lexically first. `birthtime` is in DESIGN but
not in `Attributes`, and a backend that reports one is the exception, so mtime
stands in and the tour says so.

Two overrides: a `.tungstate-keep` file beside the copy to keep, and
`--keep <path>` on the command line.

---

## What ships

**`tungstate-core::dupes`,** pure, no I/O: given a snapshot and the policy,
group by size, ask the caller for partial and full hashes through a trait
(`Digest`), and return groups with a canonical copy named and a reason for
the choice. Pure so it can be property-tested, and so the same answer can be
drawn by the CLI and the window later.

**One new plan op.** `Op::SetAside { from, to, because: Duplicate { of } }`,
where `of` is the canonical copy's path. Its reverse is a move back, which
`undo` already knows how to do. No other op changes.

**`tungstate dedupe [folder]`**, with:

- no flag: report only. Groups, sizes, what would be set aside, what would be
  reclaimed.
- `--apply`: do it, journalled as a plan, undoable by id.
- `--keep <path>`: pin the canonical copy for the group containing it.
- `--json`: the report as data, so the window can be built on it later.

**A refusal worth having.** Full hashing over a networked backend reads every
byte across the network. Over the size of one candidate set it warns and asks;
`--yes` forces it. Local disks say nothing.

## What is deliberately not in this slice

- **Near-duplicates** (perceptual hashes). DESIGN §5 already calls them
  roadmap, and they are *suggest*, never *enforce*, which is a different
  screen and a different promise.
- **Duplicate directories.** "I copied the whole folder twice" is the common
  case and deserves its own slice; it is a different grouping over the same
  hashes.
- **`hardlink` and `reflink`.** Both change what a file *is*, not where it is.
  Listed at 15+ already.
- **The window.** This slice ships the engine and the command line. A
  Duplicates view inside Organize is the next slice, and `--json` exists so
  that it costs nothing extra when it comes.
- **Cross-folder dedup.** One root at a time.

## Safety properties this slice must keep

1. **Never the last copy.** A group of *n* identical files sets aside *n - 1*.
   The canonical one is never an op's `from`. This is a property test, not a
   comment.
2. **Nothing is removed.** Set aside is a rename inside the root. After a
   dedupe, the multiset of content hashes under (folder ∪ quarantine) is
   exactly what it was before.
3. **Identical means identical.** Two files are a duplicate pair only after a
   full BLAKE3 match. Size and partial hash decide who is *worth* hashing,
   never who is a duplicate.
4. **A hash is never trusted across a change.** The cache key includes size
   and mtime; a file touched since is hashed again.
5. **Nothing moves unseen.** `dedupe` with no flags changes nothing, and
   `--apply` prints the same report first.

## Verification

- Property test: for any set of files, every content hash present before is
  present after (in the folder or in quarantine), and every group keeps
  exactly one copy in place.
- A folder with three identical videos and one different: one op, correct
  canonical, correct reclaimed figure.
- Same content, different mtimes: the older one is canonical.
- `.tungstate-keep` and `--keep` each override the tie-break, and disagreeing
  with each other is an error rather than a silent winner.
- Files that are already hard links to one inode are one file, not a duplicate
  (DESIGN §9 records this trap).
- A 5,000-file folder where every file is unique does no full hashing at all,
  proved by counting reads.
- Undo puts every set-aside copy back, and a second undo is refused.
- `cargo fmt --check`, `clippy -D warnings`, `cargo test --locked`, and CI on
  Linux, macOS and Windows.

## Risks

- **Mtime resolution.** FTP rounds to the minute (DESIGN §9). Two copies a few
  seconds apart tie on mtime and fall through to the shorter path, which is
  the right answer but must be tested rather than assumed.
- **The hash cache is a cache.** Slice 6 dropped the index because a cache
  earns its keep from hits and had no reader. This one has a reader on day
  one, but it is still the piece most likely to be wrong in a way nothing
  notices: a stale hit is a wrong answer about somebody's files. Key on size
  and mtime, and test the invalidation directly.
- **Symlinks and case-insensitive filesystems.** Two paths for one file are
  not two files. macOS is case-insensitive by default; Linux is not.
