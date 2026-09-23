# Slice 8d: files that are nearly the same

**Goal:** find the files that are not byte for byte identical but hold the same
thing anyway. A photo re-exported smaller. A video re-encoded. The same song at
two bitrates. Report them as a suggestion, show enough of each one that a person
can judge, and never act without being asked.

**Runnable outcome:** `tungstate dedupe ~/Pictures --similar` lists what looks
alike and how alike it is. The Duplicates window offers *also look for files
that are nearly the same*, draws each group with pictures rather than paths
alone, and clears what you tick.

Exact duplicates shipped in [8b](08b-dedup.md) and their window in
[8c](08c-duplicates-window.md). This slice is the fuzzy half, which DESIGN §5
has always said is a different kind of answer and needs a different screen.

---

## What is already here

- A pass that groups files, chooses a copy to keep, builds a plan, applies it
  and journals it, all of which is indifferent to how the grouping was reached.
- A digest cache in the journal keyed on (root, path) and invalidated when size
  or mtime changes, which is exactly the invalidation a perceptual fingerprint
  needs.
- A window with a scan, a progress event, a stop button, per group choice, a
  confirm sheet and `Put it back`.
- `infer` for magic bytes and `kamadak-exif` for picture metadata, both already
  compiled.

## What is missing

- Nothing computes a fingerprint that survives re-encoding.
- Nothing groups by *nearly*, and nearly is not an equivalence relation, so the
  grouping cannot reuse the hash map the exact pass uses.
- The window has never shown a picture.

---

## Decisions taken (2026-09-23)

### 1. Three kinds of file, one shape of answer

Pictures, video and sound, decided together rather than one per slice, because
the grouping, the screen and the plan are shared and only the fingerprint
differs. Text is not in it: a document saved twice with an edit is a different
document, and nobody wants a file manager with an opinion about that.

### 2. A video is fingerprinted through its soundtrack first

Decoding video frames in Rust means a C decoder, and requiring one would make
this feature absent on most machines. A re-encoded video keeps its audio
recognisable, so the cheapest honest fingerprint of a video is the fingerprint
of its sound. `symphonia` demuxes mp4 and mkv and decodes the common audio
codecs in pure Rust, so this works everywhere the app runs.

Where there is no usable audio track, and only there, the pass looks for
`ffmpeg` on the machine and samples frames with it. Where neither is possible
the file is **counted and named as unchecked**, never quietly skipped. A silent
count is how a duplicate finder tells you your drive is clean when it has not
looked.

### 3. Two tiers again, for the same reason as 8b

A fingerprint that can answer *how alike* is too big to compare against every
other file: chromaprint produces a sequence, not a number, and comparing
sequences pairwise across ten thousand songs is not a thing that finishes.

So each file gets both: a **64 bit signature** for finding candidates and the
**detail** for judging them. Candidates come out of a BK-tree, which is a tree
built on the triangle inequality holding for Hamming distance, so a search for
everything within distance 10 of a signature visits a small part of it rather
than all of it. Only the survivors are compared in detail.

This is the same shape as size then partial digest then whole file, and for the
same reason: each tier is paid for by the one before it.

### 4. Alike is not transitive, so clusters grow around a leader

If A is close to B and B is close to C, A can still be nothing like C. Joining
every pair into one blob is how these tools end up suggesting you delete a
photo that resembles nothing else in the group.

Every group therefore has a leader, and **every member is within the threshold
of the leader**, not of a chain. The leader is the best copy present: most
pixels, then largest, then oldest. That bounds how different two files in one
group can be, and it makes the leader the obvious copy to keep.

### 5. Two bands, because they mean different things

| Distance | What the screen says | What it usually is |
|---|---|---|
| 0 to 4 | the same picture, at a different size | a re-export, a resize, a screenshot saved twice |
| 5 to 10 | looks like the same moment | two frames of a burst, two edits of one shot |

The first is safe to clear and the second is usually not, and a finder that
shows them in one undifferentiated list is asking somebody to delete half a
burst. Nothing is ticked by default in either band, which is the opposite of the
exact pass, where everything is ticked because the match is proof.

### 6. Pictures on the screen

The pass already decodes every picture to fingerprint it, so a thumbnail on the
same pass is nearly free. They go in the app's own cache directory, keyed by
content, and reach the window through a URI scheme handler rather than being
stuffed into the command's answer: fifty photos as base64 is most of a megabyte
of JSON per redraw.

Video gets the frame already sampled where ffmpeg supplied one, and its own mark
where it did not. Sound gets no picture and does not need one: for audio the
useful facts are the length, the bitrate and the name.

### 7. The extras go wherever exact duplicates go

**Asked and answered by the user.** The remembered choice of *set aside* or
*send to the trash* covers both passes; there is not a second setting.

I argued for set aside only here, on the grounds that a fuzzy match plus an
irreversible delete is the one combination that can lose a photo, and the user
decided otherwise. Two things carry that concern instead of a refusal: nothing
in a fuzzy group is ticked for you, and the confirm sheet for a fuzzy group
headed to the trash says in its own sentence that this is a resemblance rather
than proof.

### 8. Local only, this slice

A perceptual fingerprint needs the whole file decoded, and there is no sampled
shortcut the way there is for a digest. Over a connection that means pulling
every video across the network, so `--similar` and the window's switch are
refused on a networked place, with a sentence saying why. Exact duplicates over
a connection are unaffected.

---

## The pass

A new crate, `tungstate-likeness`, holding the decoders and nothing else, so
the picture and audio dependencies stay out of `tungstate-core`, which is pure,
and out of everything that does not ask for them.

```rust
pub enum Kind { Picture, Moving, Sound }

pub struct Print {
    pub algo: &'static str,   // "pic1", "vid1", "snd1"
    pub signature: u64,       // what the BK-tree indexes
    pub detail: Vec<u64>,     // frames, or chroma words, for judging
}

pub fn print_of(path: &Path, kind: Kind) -> Result<Print, Trouble>;
pub fn alike(a: &Print, b: &Print) -> Option<u8>;   // None across algorithms
```

`algo` is written into the stored string as its prefix, so a fingerprint from a
future version of the algorithm fails to parse as this one and is recomputed
rather than compared. **Two fingerprints from different algorithms are never
compared**, which is the mistake DESIGN §5 warns about for BLAKE3 against MD5.

Storage is one more column on the existing `hashes` row, schema v10. The row is
already thrown away when size or mtime changes, which is exactly right for a
fingerprint too, and one row per file means one invalidation rule rather than
two.

The grouping, the choice of leader and the plan live in
`tungstate_core::dupes` beside the exact pass, take the same `Wants`, and
produce a `Found` with one more field. Everything downstream of it, the
confirm, the plan, the executor, the journal and `Put it back`, is untouched.

## The window, which this slice rebuilds around a picture

**Asked for on 2026-09-23**, with a screenshot of Gemini by MacPaw: the point
of that window is that a person can *see* the file before agreeing it is a
duplicate. A list of paths cannot be checked by a human being, and a fuzzy
match that cannot be checked by a human being has no business existing.

So the results screen from 8c is replaced, for both passes at once, by three
panes:

**Left, what kind of thing.** *Exact duplicates* over *Similar files*, each
split by kind with the space each one holds: pictures, video, sound, documents,
archives, applications, folders, everything else. This is the first screen in
the app that answers "where did my disk go" before it answers "what should I
delete". Kinds come from the mime `tungstate-attrs` already reads, plus two
special cases macOS needs: a `.app` is an application rather than a folder, and
a folder group is its own kind.

**Middle, the groups.** One row per group: a small picture of the copy being
kept, its name, its size, and how many of its copies are ticked out of how many
exist. The row opens to show every copy with its own checkbox, its full path
and its date.

**Right, the file.** The highlighted copy, as large as the pane allows. A
picture for a picture, the sampled frame for a video, and for everything else
its name, size, date and where it lives. The header says how many copies are
ticked and how many were found.

**The foot.** What was found in total, what is ticked, and the one action
button, which already says which action it will take.

Below about 1000 points of width the preview folds into a toggle rather than
squeezing three panes into 860, which is the same rule the transfer panes use.

### The selection model changes with it

8c ticked a whole group and let you nominate which copy survived it. That
cannot express what this window shows, where four copies each have a checkbox
and you may keep two of them.

So the seam takes **paths, not groups**: the window sends the copies it wants
dealt with, and the engine refuses any request that would leave a group with
nothing, rather than trusting the window to have counted. The property that
every group keeps one copy moves from being something the UI arranges into
something the engine will not let the UI get wrong.

### Choosing without ticking four hundred boxes

Selecting by hand is the point of the preview, and it is not how anybody deals
with three thousand files. So the same list carries rules, and every rule is
just a way of setting the ticks that you can then correct one by one:

- keep the newest copy in every group, or the oldest;
- keep the biggest copy, which for a resized photo or a re-encoded video is the
  one worth keeping;
- keep the copy that sits in a folder you name, for "my library is the real
  one and the rest are strays";
- tick everything, or nothing.

A rule never acts. It moves the checkboxes and leaves the list on screen, so
the preview still gets the last word, and the foot still shows what is about to
happen before the button is pressed.

The one rule that stays automatic is the safety one: **no rule may empty a
group.** Each of them keeps a copy by construction, and the engine checks again
anyway.

## What the window must never do

1. **Never tick a resemblance for you.** The exact pass ticks everything
   because it has proof. This one has a guess.
2. **Never mix the bands.** *The same picture at a different size* and *looks
   like the same moment* are separate sections with separate wording.
3. **Never say a file was checked when it was not.** Unreadable, unsupported
   and unchecked are counted and named.
4. **Never lose the last copy**, and never drop the leader, which holds here
   exactly as it holds for the exact pass.
5. **Files are not operations**, still.

## What is not in this slice

- **Text.** See decision 1.
- **Cross-folder comparison.** One root at a time, as the exact pass has.
- **Scanning a connection.** Decision 8.
- **Faces, scenes or anything that needs a model.** This is arithmetic on
  pixels and on sound, and it can explain every answer it gives.
- **Re-encoding anything.** The app finds the extra copy. It does not make one.

## Verification

By hand, on files made for it: a photo and its 50% resize, the same photo
re-saved as JPEG at a lower quality, two frames of a burst, a photo that
resembles nothing, a video and its re-encode at a lower bitrate, a silent video,
a video whose audio codec `symphonia` cannot read on a machine with ffmpeg and
on one without, an MP3 and a FLAC of one song, and a folder with none of the
above. Both window sizes and the trash path as well as the set-aside path.

Property tested: every group keeps one copy, every member is within the
threshold of its leader, and no file appears in two groups.

`cargo fmt`, `clippy -D warnings`, `cargo test`, `npm run build`, and CI on
Linux, macOS and Windows.

## Risks

- **Decoding files somebody else made.** Every decoder is a parser, and a
  parser is where a malformed file does something surprising. The workspace
  forbids `unsafe`, the decode is bounded by explicit width, height and memory
  limits, and a file that fails to decode is counted rather than fatal.
- **The slowest thing the app has ever done.** Exact duplicates mostly read
  nothing, because most sizes are unique. This reads and decodes every picture
  and every video. The cache makes the second scan cheap and does nothing for
  the first, which is why the switch is off unless asked for and the progress
  event says which kind of file it is on.
- **Thresholds are taste.** 4 and 10 are the numbers the literature and every
  other tool converge on for a 64 bit difference hash, and they are still
  guesses about somebody's photos. They are constants with names, in one place,
  and the screen shows the percentage so a wrong call is visible rather than
  silent.
- **ffmpeg is somebody else's program.** It is found, never installed, never
  bundled, and its absence costs one case rather than the feature.
