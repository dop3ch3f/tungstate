# Slice 13b tour: tidy on storage that cannot rename

FTP and S3 can now be tidied in place too. Neither can rename (OpenDAL's FTP
service refuses to, and an object store has no such thing), so a move there
is a copy, a check that the copy is whole, and a delete. The 13a refusal is
gone. The brief is [13-tidy-in-place.md](13-tidy-in-place.md); the first half
is [13a-tour.md](13a-tour.md).

---

## 1. A copy, on the server where it can be

```rust
fn copy(&self, from: &Path, to: &Path) -> Result<()> {
    let mut reader = self.open_read(from)?;
    let mut writer = self.create_write(to)?;
    std::io::copy(&mut reader, &mut writer).map_err(...)?;
    writer.finish()
}
```

- **A new defaulted trait method.** `Backend::copy` streams the bytes
  through this computer, which is correct for every backend.
- **The OpenDAL adapter overrides it where its service copies on the
  server.** S3's `CopyObject` copies inside the bucket, so nothing comes
  down. FTP has no copy, so every byte comes down and goes back up.
- **The adapter repeats the default's five lines in its fallback.** Rust
  can't call a trait's default from inside an override of that same
  method, so the adapter spells them out.
- **`copies_on_server()` says which happens**, so the preview can tell you
  before you start.

## 2. Moving without a rename

`move_file` in the executor is now the one place a tidy moves a file:

1. If the storage can rename, rename, as before.
2. Otherwise:
   - copy;
   - check the copy is the same size as the original;
   - delete the original.

   A copy of the wrong size is removed, and the original kept.

The original is deleted only after the copy has been checked, the same
order a drain uses. A folder moved whole, such as a photo library kept as
one unit, is refused on such storage. It would be copied file by file, and
nothing could undo that halfway.

Undo goes through the same `perform`, so putting a tidy back is copies too.

## 3. A crash between the copy and the delete

Every operation is written to the journal as begun before it happens, and
marked done after. A crash in between leaves it begun, and the next run's
recovery looks at what is on disk.

For a rename that can only mean "before" or "after". For a move by copy
there's a third state, both files present, which is exactly what stopping
between the copy and the delete leaves. Recovery now settles it:

- **The copy is the same size as the original:** the copy finished, so the
  original is deleted and the move counts as done.
- **The copy is shorter:** it was cut off, so the copy is deleted and the
  move counts as never having happened.

Only where the storage can't rename. Where it can, both files means
something else happened, and Tungstate still leaves both alone rather than
guess. `both_files_after_a_rename_is_still_left_alone` holds that line.

## 4. The cost, said first

The preview says what a tidy here will copy:

> This storage cannot rename, so each file comes down to this computer,
> goes back up, and then the original is deleted: 1.0 GB each way.

The same sentence goes into the "Tidy …?" question. `tungstate plan`
prints a note with the same numbers. On S3 the sentence says the server
makes the copies.

## 5. What is checked

- **Executor**, with a local backend that says it can't rename:
  - a tidy and its undo by copy;
  - a cut-off move with a whole copy is finished;
  - a cut-off move with a short copy is taken back;
  - both files after a real rename are left alone.
- **Against FTP:** a folder is planned (the note says the bytes come
  down), tidied and put back, and the files are checked on the server.
- **Against S3 (SeaweedFS):** the same, with the bucket making the copies.
- **The whole suite** in parallel and one at a time, the FTP, S3 and SMB
  suites, and the window's checks.

## What is not verified

- **Speed on a real NAS over FTP.** A tidy there sends every moving file
  down and back up. The preview says so, but nobody has timed a big one.
- **Folders kept as one unit** (`opaque` in the rules) can't be moved on
  FTP or S3. They're reported as failed moves, with the reason, rather than
  copied file by file.
