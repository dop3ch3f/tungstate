# Slice 8d tour: files that are nearly the same

A photograph exported at half the size shares no bytes with the original. This
is the slice that finds it anyway, shows you both, and lets you decide.

Brief: [08d-near-duplicates.md](08d-near-duplicates.md). The exact half is
[8b](08b-dedup.md) and its window is [8c](08c-duplicates-window.md).

---

## 1. Everything the exact pass leans on is gone

The pass in slice 8b is cheap because of one fact: two files of different
lengths cannot be identical. In an ordinary folder most lengths are unique, so
most files are never opened at all.

None of that survives here. A re-export is a different length **by design**.
So every candidate has to be opened, and then every candidate compared against
every other one, which is `O(n²)` and is fine for a thousand files and is not
fine for a hundred thousand.

The answer is the same shape 8b uses for reading, applied to comparing:

1. Every file gets a **64 bit signature**. A tree holds them and answers
   *everything within 12 bits of this* by visiting a small part of itself.
2. Only what that returns is compared in **detail**, which is the accurate and
   expensive answer.

## 2. A video is fingerprinted through its soundtrack

Decoding video frames in Rust means a decoder written in C. Requiring one would
make this feature absent on most machines, and bundling one is a licensing
conversation nobody asked for.

A re-encoded video keeps its sound recognisable, and `symphonia` reads the
audio track out of an mp4 or an mkv in pure Rust. So **the cheapest honest
fingerprint of a video is the fingerprint of what it sounds like**, and it
works everywhere the app runs.

Frames are the fallback, not the plan. A silent video, or one whose audio codec
is not among those compiled in, is offered to `ffmpeg` if the machine has it.
The same fallback covers the pictures the pure Rust decoders cannot read, which
in practice means an iPhone's HEIC, and the frame that comes back is hashed by
exactly the same arithmetic as an ordinary JPEG, so the two are comparable.

Where neither works the file is **counted and named**. A finder that quietly
passes over what it cannot read tells you your drive is clean when it has not
looked.

## 3. A BK-tree, which is one idea

```rust
struct Node {
    signature: u64,
    label: usize,
    children: BTreeMap<u32, usize>,
}
```

Every node keeps its children **under the distance from itself**. A search that
has reached a node sitting `d` away from what you asked about need only walk
the children numbered `d - radius` to `d + radius`, because Hamming distance
obeys the triangle inequality: anything closer than that to the target would
have to be closer than that to the node too, and it is not.

That is the whole trick. The rest of the tree is skipped without being looked
at.

**A tree that misses answers is a duplicate finder that says your drive is
clean**, so it is checked against the slow version: a property test builds a
tree from up to sixty random signatures and asserts that `within` returns
exactly what a plain scan of all of them would, at every radius from 0 to 64.

## 4. Alike is not a relation that chains

A can resemble B and B resemble C while A and C have nothing in common. Joining
every close pair into one blob is how these tools end up suggesting somebody
delete a photograph that looks like nothing else in the group.

So every group has a **leader**, and every member is measured against the
leader, never against a neighbour. That bounds how different two members can
be, and it makes the leader the obvious copy to keep.

Which copy leads is only knowable once everything has been looked at, so the
pass runs in two halves: fingerprint everything, sort by how much of the thing
there is, then cluster in that order. The copy with the most pixels meets the
clustering first and therefore leads by construction rather than by a rule
applied afterwards.

## 5. Two bands, and the numbers behind them

| How alike | What the screen says |
|---|---|
| 90 and over | the same thing, in a different wrapper |
| 70 to 89 | looks like the same moment |

Measured on real photographs rather than guessed, with
`cargo run -p tungstate-likeness --example score`, which is in the repository
so the numbers can be checked again when the arithmetic changes:

| Pair | Alike | Signatures apart |
|---|---|---|
| a photo and the same photo at 400px | 100 | 1 bit |
| a photo and the same photo at awful JPEG quality | 100 | 0 bits |
| a photo and a 15% crop of itself | 49 | 8 bits |
| two unrelated photographs, worst of 45 pairs | 36 | 19 bits |

Two things fall out of that, and both are written down rather than smoothed
over. The gap between an honest match and an honest miss is enormous, which is
why the thresholds are not delicate. And **a crop is not found**: at 49 it sits
nearer the unrelated pairs than the real ones, and pulling the threshold down
to catch it would start offering people unrelated photographs. A crop is a
picture somebody deliberately changed, so missing it is the right failure.

A group holding both a re-export and a photograph of one moment makes two
claims of different strengths, so it is split into two rows. Dragging the safe
claim down to the weaker wording would be the easy thing and the wrong one.

## 6. The seam now takes paths, and refuses

8c ticked a whole group and let you nominate the survivor. The window this
slice ships has a checkbox on every copy, and no description in terms of whole
groups can say *keep two of these four*.

So `clear_duplicates` takes copies. And because the window is not the only
thing that can call it, the rule moved with it:

```rust
let Some(instead_of) = staying.first() else {
    return Err(Trouble::WouldEmpty { group: bundle.id.to_string() });
};
```

**"Every group keeps one copy" stopped being something a screen arranges and
became something the engine will not let a screen get wrong.** Both passes feed
the same function, which is why one rule covers both.

## 7. The window, rebuilt around being able to see the file

Asked for with a screenshot of Gemini by MacPaw. The point of that window is
that a person can *look* at the file before agreeing it is a duplicate, and a
fuzzy match nobody can check has no business existing.

Three panes. On the left, what kind of thing and how much of it, which is the
first screen in this app that answers *where did my disk go* before it answers
*what should I delete*. In the middle, one row per group, opening into its
copies. On the right, the file itself, as large as the pane allows.

Below about 1000 points of width the preview folds into a toggle rather than
squeezing three columns into the window's own 860 minimum.

**Nothing in a fuzzy group is ticked for you.** The asymmetry is deliberate: an
identical group has proof, so the work is already done and unticking is the
exception; a resemblance is a guess, and a guess that arrives pre-agreed is how
somebody loses a photograph. The confirm sheet says so in its own sentence
whenever a ticked copy came from a resemblance.

Selecting by hand is the point of the preview and is not how anybody deals with
three thousand files, so the same list carries rules: keep the newest, the
oldest, the biggest, tick every extra, tick nothing. **A rule never acts.** It
moves the checkboxes and leaves the list on screen, so the preview still gets
the last word, and every one of them keeps a copy by construction.

## 8. Pictures arrive over a scheme of their own

```rust
.register_uri_scheme_protocol("thumb", move |_ctx, request| {
    serve_picture(serving.as_ref(), request.uri().path())
})
```

Not inside the command's answer: fifty photographs as base64 is most of a
megabyte of JSON on every redraw, and this is a file on disk being asked for by
name. The name is checked to be one path component with no dot-dot in it,
because serving anything else would let the window read any file it can spell.

The pictures themselves are nearly free. The pass has already decoded every
photograph in order to fingerprint it, so a thumbnail is one more resize on a
decode that has happened anyway. They live in the operating system's own cache
directory, where deleting the lot costs the next scan a redraw and nothing
else.

## 9. The cache is the whole feature, the second time

An exact pass over an untouched folder is cheap because most files are never
read. A fingerprint pass has to decode every picture and every video the first
time, and nothing makes that cheaper.

So every fingerprint goes into the journal beside the digest, in the same row,
keyed by the path and checked against size and mtime. One row per file means
one invalidation rule, and that rule was already written and already tested.

```
looked at 7 file(s): 7 opened, 0 remembered from last time
looked at 7 file(s): 0 opened, 7 remembered from last time
```

The stored value names the algorithm that produced it, so a print written by a
later version of the arithmetic simply fails to parse as this one and is taken
again. That is the migration for a change to the maths, and it costs nothing.

## 10. Driving it by hand found two things the harness could not

The headless harness photographs any screen, which is how the layout and the
smallest window size were checked. Run against a folder of real photographs,
the live window showed two defects it cannot see:

1. **An exact duplicate of a photograph showed no picture, while a resemblance
   did.** Only the second pass makes thumbnails, and it skips what the first
   pass already accounted for. The copy a group *keeps* is fingerprinted
   either way, and every copy in an exact group is the same bytes, so it is the
   same picture. One lookup fixed it.
2. **Put it back did nothing, and said nothing.** It went through the folder
   half's undo, which walks up to a `.tungstate/policy.toml` and refuses
   without one; this window scans folders that have none, by design. An undo
   that needs rules the scan never needed is an undo that is missing exactly
   when somebody wants it. The Duplicates window now has its own, and the
   result screen shows a failure instead of swallowing it.

The second one predates this slice. Slice 8c's walk happened to use a governed
folder, so it passed.

What the same walk confirmed, on real files: three renditions of one photograph
grouped with the biggest leading, an unrelated photograph left alone, a crop
correctly not offered, a corrupt JPEG named rather than skipped, counts that
follow the ticks, the resemblance sentence appearing in the confirm sheet only
when a resemblance was ticked, three files set aside, and everything returned
by Put it back.

## 11. What is checked

- **10 tests in `tungstate-likeness`**, against drawn pictures and a
  hand-written WAV, so they run on any machine: a photograph saved smaller is
  the same photograph, two different photographs are not confused, one tune at
  two sample rates is one tune, a print survives being written down, and a
  print from another algorithm is never read back.
- **10 in `tungstate_core::similar`**, including the property test that the
  tree agrees with a plain scan, and one that no group ever swallows its own
  leader.
- **6 in `tungstate-execute`**, against real files: the second look decodes
  nothing, a changed picture is looked at again, and a recalled fingerprint
  still gets its picture.
- **4 more in `tungstate_core::dupes`** for the refusal, and **9** in the
  window's seam, including the undo that needs no rules.
- Plus `npm run build` (the CSS checker and `vue-tsc`), `cargo fmt`,
  `clippy -D warnings`, 536 tests, and CI on Linux, macOS and Windows.

The CSS checker earned its keep again: it caught an `aria-expanded` bound with
no rule drawing it, which is a state the screen claims to have and does not
show.

## What is still not verified

- **No real video has been through it end to end.** The soundtrack path is
  exercised by tests on generated audio and the frame path by tests that skip
  when ffmpeg is absent, but nothing has pointed the window at a folder of
  holiday footage and a re-encode of it. This is the headline case in the
  syllabus and it is the least proven thing here.
- **No real music library.** Chromaprint matching is tested on a hand-written
  tune at two sample rates, which is a weaker claim than an MP3 against a FLAC
  of one recording.
- **Only the retro theme.** The three panes have not been looked at in graphite
  or paper, which now joins the same list slice 8c left.
- **Nothing large.** The biggest folder scanned is eleven files. The brief said
  memory on a very large scan would be measured, and it has not been. A cluster
  holds every copy's path and every detail fingerprint, and a chromaprint of a
  two-hour video is not small.
- **HEIC has not been tried**, on a machine with ffmpeg or without one.
