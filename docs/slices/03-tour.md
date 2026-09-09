# Slice 3 tour: the durable drain

Read this next to the diff. This is the slice the project exists for, so it is worth reading properly.

Files: `crates/tungstate-transfer/src/lib.rs` is the engine, `walk.rs` finds and orders files, `conflict.rs` decides what happens when a name is taken. The journal gained a `links` module and migration v2. The CLI gained `link add`, `link list` and `link run`.

---

## Part 1: The ordering, which is the entire safety argument

Everything else is detail. This sequence is why the drain can be killed at any instant without losing a file:

1. Journal the intent.
2. Stream to a temporary name at the destination, hashing while copying.
3. Verify.
4. Rename the temporary file into place.
5. Journal the outcome.
6. **Only then** touch the original.

Read it as a list of "what if we die here?" questions:

- Die after 1: a row says `intended`, nothing else happened. Next run cleans up and redoes it.
- Die during 2: a `.part` file exists, the original is untouched. Next run deletes the partial and redoes it.
- Die after 4 but before 5: the destination has the file, the journal still says `intended`. Next run redoes the copy, notices the destination already holds identical content, and moves on.
- Die after 5: the journal says committed, the original still exists. Next run sees the destination matches and removes the original.

At no point does a source get deleted without a verified, journaled copy existing. The worst case is always a redundant copy, never a lost file. That asymmetry is deliberate and is why step 6 is last.

**This was tested for real, not just in unit tests.** A 400MB file, `kill -9` mid-copy, an 81MB partial left on disk, both source files intact. Resume detected the interrupted operation, deleted the partial, re-copied, and the result was byte-identical to the original, verified with `b3sum` rather than with tungstate's own code.

---

## Part 2: Lifetimes on a struct, at last

```rust
pub struct Transfer<'a> {
    link: &'a Link,
    source: &'a dyn Backend,
    destination: &'a dyn Backend,
    journal: &'a Journal,
    resolver: &'a mut dyn ConflictResolver,
    progress: &'a mut dyn Progress,
}
```

Slice 1 had `&'static str`, the easy lifetime. This is the real thing.

`Transfer` holds six borrowed things and owns none of them. `'a` is a name for "however long those borrows are valid", and putting it on the struct tells the compiler that a `Transfer` may not outlive any of them. Without it, you could build a `Transfer`, drop the `Journal`, and then use a dangling pointer. The lifetime is what makes that a compile error.

The alternative was to have `Transfer` own everything, which would mean cloning the `Link` and moving the `Journal` in. Borrowing is right here: a `Transfer` is one run, and the journal outlives many runs.

**`&mut dyn Trait` is the other new shape.** `resolver` and `progress` are mutable trait objects, because both need to remember things: the resolver caches an "apply to all" answer, and progress implementations accumulate. Same dynamic dispatch idea as `Box<dyn Backend>`, but borrowed rather than boxed, because the caller owns them.

---

## Part 3: One read, two results

```rust
loop {
    let read = reader.read(&mut buffer)?;
    if read == 0 { break; }
    hasher.update(&buffer[..read]);
    writer.write_all(&buffer[..read])?;
    written += read as u64;
}
```

The most important loop in the codebase, and it is deliberately dull.

A 400GB drain is bounded by how fast bytes move. Reading the source twice, once to copy and once to hash, would double the expensive part. So one pass fills the buffer, feeds the hasher, and writes to the destination.

**`&buffer[..read]` matters.** `read()` returns how many bytes it actually got, which is often less than the buffer. Hashing or writing the whole buffer would include stale bytes from the previous iteration. This is the classic buffered-IO bug and the slice notation is what avoids it.

**The buffer is one megabyte, allocated once outside the loop.** Allocating per chunk would mean four hundred thousand allocations for a 400GB file.

```rust
drop(writer);
```

An explicit `drop` with a comment, because it looks redundant and is not. Dropping closes the file handle, and Windows refuses to rename a file that still has one open. Without this line the code passes on macOS and Linux and fails in CI, which is the same shape of bug as slices 0 and 1.

---

## Part 4: A macro, and when writing one is justified

```rust
macro_rules! string_enum {
    ($type:ty { $($variant:ident => $text:literal),+ $(,)? }) => { ... };
}

string_enum!(VerifyLevel { Size => "size", Hash => "hash", Readback => "readback" });
```

Four enums each needed `as_str` and `parse`, which is the same twelve lines written four times. A declarative macro generates them.

**The rule for reaching for a macro:** when the repetition is structural rather than incidental, and when the generated code is obvious from the call site. Here the call site reads as a table of variant-to-string, which is exactly what it is. If you had to read the macro body to know what `string_enum!(Order { ... })` produced, it would be the wrong tool.

The payoff beyond brevity: the same spelling serves the database and the command line. A value stored in SQLite is always a value you could have typed as `--verify`, and vice versa, because there is one source of truth for both.

`macro_rules!` is pattern matching over syntax. `$variant:ident` captures an identifier, `$text:literal` a literal, `$(...),+` means one or more separated by commas, and `$(,)?` allows a trailing comma. That is most of what you need to read the macros you will meet.

---

## Part 5: The conflict resolver, and designing for absent humans

```rust
pub trait ConflictResolver {
    fn resolve(&mut self, conflict: &Conflict) -> ConflictAction;
}
```

Two implementations. `InteractiveResolver` prompts and remembers. `FixedResolver` always answers the same way.

That split exists because of a requirement that is easy to miss: **the drain's whole purpose is running while nobody is watching.** A prompt is right when you are at the terminal and catastrophic when you are not, because one awkward filename would leave the drain frozen for hours while you are out.

So `run_link` decides which to use:

```rust
let interactive = !yes && on_conflict.is_none() && std::io::IsTerminal::is_terminal(&std::io::stdin());
```

If stdin is not a terminal, meaning the run is backgrounded, piped, or under a daemon, nothing prompts and the link's configured action applies. Quarantine by default: the incoming file is preserved under `.tungstate-quarantine/`, the source is removed so space is still reclaimed, and the count is reported at the end.

**The interactive resolver also handles input simply running out**, which happens if a terminal closes mid-run. `read_line` returning `Ok(0)` means end of input, and it falls back rather than looping forever on an empty read. There is a test for exactly that, because an infinite loop in a tool you left running overnight is a bad failure.

**One deliberate asymmetry in the prompt.** Lowercase `r` is rename, capital `R` is replace. They differ enormously in consequence, and giving them the same letter with a modifier would be a trap. Replace has to be typed deliberately.

**And replace does not destroy anything.** The file being replaced is moved into quarantine first. "Replace" is the user saying which copy they want in the canonical place, not permission to delete the other one. That distinction is in the code with a comment, and it has a test.

---

## Part 6: Struct update syntax and `let ... else`

Two small pieces of modern Rust worth naming, both in this slice.

```rust
let mut summary = Summary {
    recovered: self.recover()?,
    ..Summary::default()
};
```

`..Summary::default()` fills every remaining field from the default. Useful when a struct has many fields and you care about one. You will see it constantly in Rust codebases.

```rust
let Some(modified) = file.modified else {
    return false;
};
```

`let ... else` handles the failure case and diverges, leaving the happy path unindented. The alternative is `match` or `if let` with the real logic nested inside, which drifts rightwards. Use it whenever "if this is absent, bail" is the whole story.

```rust
.is_ok_and(|age| age < self.link.cooldown)
```

`is_ok_and` on a `Result` means "is this Ok, and does the value satisfy this?" without unwrapping. Reads better than matching for a single boolean question.

---

## Part 7: Where clippy was right about my design

Clippy flagged two arms of the verification match as identical:

```rust
VerifyLevel::Size | VerifyLevel::Hash => Ok(()),
```

It was correct, and the honest fix was to merge them and explain why rather than to contrive a difference. `Size` is satisfied by the length check just above it. `Hash` adds nothing on top *today*, because the digest is computed during the single read the copy already needs, so there is no cheaper mode to fall back to.

They separate in slice 12, where a remote backend can offer a server-side checksum for `Hash` to compare against and `Size` will not ask for one. Until then the distinction is recorded intent, not behaviour, and the comment says so. Pretending otherwise would be worse than the duplication.

`Readback` is the level that genuinely differs now: it re-reads the whole file from the destination and compares digests. It is the only level that proves the bytes on the far disk are the bytes you sent, and it costs a second full read.

Clippy also caught `digest` taking `&self` without using it, which became a free function, and a `match` that should have been an `if let`. Neither was a bug; both made the code slightly wrong to read. That is what the pedantic lint set is for.

---

## Part 8: Two UX bugs that only showed up in a real run

The unit tests all passed before the binary was ever run against real directories. Then the first real drain printed this:

```
skipped
skipped
skipped
```

Three problems in one. No filenames, because `progress.starting()` was called *after* the cooldown check returned early, so skipped files got a `finished` with no matching `starting`. And no reason, because `FileOutcome::Skipped` carried none.

The fix moved `starting()` to the top of `transfer_one`, so every file gets a matched pair, and split the outcome into `Skipped(SkipReason)`:

```
  big-video.mp4 (2.9 MiB)... skipped (written too recently; will move next run)
```

The lesson is not about progress reporting. It is that a test suite proves the logic and tells you nothing about whether the tool is usable. Run the thing.

---

## Part 9: What the walk does and does not do

`walk::files` collects every regular file, depth first, into a `Vec`.

**Collecting is defensible here**, even though slice 1 refused to do it. Ordering by size requires the whole list before the first transfer starts, so streaming would buy nothing. The list is bounded by file *count*, not content, so a drain of enormous videos is cheap to plan. A million-file folder would want the streaming walk that slice 6 builds.

**Symlinks are skipped, and that connects back to slice 1.** `Backend` already refuses to read through a link. If the walk followed them, a drain pointed at your home directory could copy content from anywhere the link reached, and then delete the link as though it had moved the content. Skipping is the honest behaviour until slice 5 introduces a symlink policy.

**Pruning only happens when the drain actually removed things.** A `--copy` run has emptied nothing, so anything empty was already empty and is none of our business. Directories are pruned deepest-first, so emptying a child lets its parent go in the same pass, and only if genuinely empty.

---

## Part 10: Migration v2, and a schema that had to change

The journal gained a `links` table and the `ops` table gained a `link_id` column:

```sql
ALTER TABLE ops ADD COLUMN link_id INTEGER REFERENCES links (id);
```

`link_id` exists so a resumed drain finds *its own* interrupted work. Without it, `incomplete()` returns every interrupted operation on the machine, and running one link would clean up another's in-flight transfers. There is a test named `interrupted_work_is_scoped_to_its_own_link` that fails if that column is ignored.

The migration runner from slice 2 handled this without any changes, which is the payoff for building it then rather than hardcoding a schema. `migrating_an_existing_v1_journal_preserves_its_rows` proves the upgrade path a user with an existing journal actually takes.

**This widened the journal crate's remit** from "provenance" to "persistent state", which is a scope change worth naming rather than letting drift. The alternative was a separate config file, which would have split a link's definition from its progress across two stores with no transaction between them.

---

## Verified for real

Beyond the 54 tests:

- A drain of three files across a nested tree. Source emptied to `0B`, tree mirrored, directories pruned.
- Every recorded digest independently recomputed with `b3sum` and compared. All matched.
- `kill -9` mid-copy of a 400MB file. Partial left, sources intact, resume completed, output byte-identical to the originals.
- The cooldown correctly held back files written seconds earlier.
- Omitting `--move`/`--copy` exits 2 with an explanation rather than guessing.

---

## What to look at in review

- **The ordering in Part 1.** If you disagree with any step's position, say so now; everything downstream assumes it.
- Is quarantining the replaced file right, or should `replace` mean replace?
- The cooldown is 30 seconds. Too long for a fast drain, too short for a slow download?
- `run()` returns `Err` on the first file that fails hard, abandoning the rest. Should one unreadable file stop a drain of four thousand, or should it be recorded and skipped? I think the latter, and slice 4 is where per-file error policy belongs, but it is a real question now.
