# Slice 8g tour: photos that look alike, on the NAS

"Also find files that are nearly the same" used to refuse a folder on a
connection: it works out what a photo looks like by opening the whole
picture, and over a network that means downloading every photo. It now
works there, and a typical photo costs its first 66 KB rather than all of
it. The near-duplicate pass itself is [08d-tour.md](08d-tour.md).

---

## 1. The preview inside a photo

Cameras and phones store a small preview picture, around 160 by 120, in a
JPEG's EXIF, which sits at the very start of the file. The difference hash
shrinks every picture to 17 by 16 pixels before it compares anything, so a
preview carries all the detail the hash ever uses.

`tungstate_likeness::preview::from_head` takes the first bytes of a file:
1. finds the EXIF (`kamadak-exif`, already in the workspace for Organize);
2. follows `JPEGInterchangeFormat` and its length in the thumbnail section
   to the preview's bytes;
3. decodes them, with a memory limit, as every decode here has.

It reads 66 KB, because EXIF lives in one APP1 segment, which can't be
longer than 64 KB, right after the file's first two bytes.

**The same algorithm name as a full decode, on purpose.** A camera original
on the NAS is printed from its preview. The smaller export of it that you
want to find has no preview, and is printed from all its pixels. They have
to be comparable, or the main case this feature exists for is missed. The
test `a_preview_gives_the_same_look_as_the_whole_photo` checks that a
preview's print and the full photo's agree.

## 2. Two things a preview would get wrong

- **The photo's size.** The print's weight decides which copy leads a group,
  so you keep the biggest copy. A preview is 160 pixels whatever the photo
  is, so the weight comes from the EXIF's `PixelXDimension` and
  `PixelYDimension` instead.
- **Black bars.** Some cameras letterbox a 3:2 photo into a 4:3 preview.
  Bars change the shape the hash shrinks, so `without_bars` trims runs of
  near-black rows and columns from the edges. Bars that would take more
  than a third of the height or width aren't bars, they're a dark photo,
  and are left alone. Writing the test for the night sky caught the first
  version, which kept only the middle third of an all-black picture.

## 3. The eye reads through a connection

`Eye` (in `tungstate-execute`) now has two sources:
- **`Eye::new(path)`:** a folder on this machine, decoded where it is, as
  before.
- **`Eye::over(backend, target)`:** a folder on a connection.
  - A picture: read its first 66 KB and print it from its preview.
  - No preview, or a video or sound: listed in `waiting()` with its size,
    and reported as not compared yet.
  - After `fetching_the_rest()`: downloaded to a temporary file, keeping
    its extension (the sound and video decoders go by it), fingerprinted,
    and the file is removed when done.

Every print still goes into the journal as before, keyed by the folder's
target, so the second scan of a NAS folder costs nothing, previews
included.

Over a connection the scan also stops sniffing every file's first 8 KB to
learn its kind; the name says it. Those reads were exactly the per-file
round trips 8e and 8f took away.

## 4. Asking before the rest comes down

- **Window.** When files are waiting, Duplicates says how many, what kind
  ("screenshots, edited exports, videos") and what downloading them costs,
  beside a "Download and compare" button. It scans again with permission to
  fetch. The previews are remembered by then, so only the rest crosses the
  network.
- **Command line.** On a mounted share, `tungstate dedupe --similar` asks
  the same question at a terminal. `--yes` agrees in a script; with no
  terminal and no `--yes`, it says what was skipped and how to include it.

## 5. A bug this found

An `fs` connection (a folder on this machine reached through the connection
factory) handed the eye its target, `nas:photos`, as if it were a path on
this machine. That names nothing, so "looks alike" quietly found nothing
there.
- **The fix:** anything named `connection:folder` is looked at through the
  connection, and a connection that isn't networked reads whole files
  straight away.
- **The regression test:**
  `a_folder_on_a_connection_is_looked_at_through_it_not_by_its_name`. It
  fails without the fix.

It has its own commit.

## 6. What is checked

- **Previews:**
  - a camera JPEG's preview prints like the full photo, weighed as the
    photo;
  - bars are trimmed and a dark photo isn't;
  - a file without a preview has none.
- **The eye over a connection:**
  - a camera photo is compared from its preview, with nothing waiting;
  - a PNG waits, with its size, until the download is agreed, and is then
    remembered.
- **The `fs` connection regression above.**
- **The whole suite** in parallel and one at a time, and the window's
  checks.

## What is not verified

- **Your NAS's photos.** Real cameras put previews in slightly different
  places; the code follows the EXIF standard's pointers. iPhone HEIC files
  keep theirs elsewhere, so they wait for the download like PNGs.
- **Over real SMB.** The eye's connection path was tested with a local
  folder standing in for a share; reading the first bytes over SMB is
  tested in the SMB suite on its own.
