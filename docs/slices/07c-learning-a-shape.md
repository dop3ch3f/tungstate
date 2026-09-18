# Slice 7c: learning a shape a folder already has

**Goal:** let somebody who has *already* organised a folder keep that
organisation, without describing it again in a language they have just met.

**Runnable outcome:** `tungstate folder learn ~/Media` reads the folder, says
what shape it is in, writes that shape down as rules, and says what would make
it better — with the counts that justify each suggestion. `--write` saves it.
Nothing moves.

**This brief is the spec.** The tour is in `07c-tour.md`.

---

## Why this exists

Every slice so far runs one way: rules in, plan out. That is the right
direction for a folder with no shape, and the wrong one for the folder most
people actually have — a `Media` directory they have been filing by hand for
years and would like to keep filing that way without the clicking.

Slice 7b's own walkthrough made the cost concrete. Pointed at a folder that
already had four levels of hand-made structure, the `documents` starting layout
planned this:

```
Work/Clients/Acme/2023/invoice-0012.pdf  ->  PDF/2023/invoice-0012.pdf
Personal/Photos/Japan 2023/DSC_0001.jpg  ->  Unsorted/DSC_0001.jpg
```

…and then `rmdir`'d `Personal`, `Projects` and `Work`. Every canned layout in
the product does this, because every one of them describes a shape rather than
reading one. For a folder that was already tidy, tidying it is *lossy*.

## Why it is 7c and not part of 8

Slice 8 is dedup, `on_duplicate` and `Trash` (DESIGN §5, and the syllabus entry
that amended it). This needs none of that and blocks none of it.

---

## The shape to aim at

The yardstick is the structure the user actually keeps:

```
YEAR / APP / TYPE / EXTENSION / SIZE-GROUP
2024 / WhatsApp / Photo / jpg / under-100MB
```

Five levels, and the interesting thing about them is that they are not all the
same kind of thing:

- **year**, **extension** and **size-group** are computable from the file —
  `{date:%Y}`, `{ext}`, and a `bucket` on `size` (slice 5 already spells those
  labels in words, so `under-100MB` rather than `<100MB`, which is not a legal
  name on Windows or over SMB).
- **type** is computable too, from `mime`.
- **app is not.** Nothing about `IMG-20240312-WA0001.jpg` says WhatsApp. It is
  knowable only from *where the file is*.

That last one is the whole design problem, and §1 of the tour is about it.

## Scope

**In:**

- `tungstate-core::learn` — I/O-free, like the rest of core. Snapshot in,
  rules out.
- Two policies out, not one: the shape **as it stands**, and the same shape
  **improved**. Being able to read both and choose is the point; a suggestion
  nobody can see is just an opinion.
- Suggestions carry the counts that justify them, never a bare opinion.
- `tungstate folder learn <PATH> [--write] [--improved]`.

**Out:**

- The window. The layout picker should grow a fifth card — *keep the shape it
  already has* — and show both rule sets in the preview, and that is slice 7d.
  The command line is the reference surface and goes first, as it did for 5/5b
  and 7/7b.
- Renaming files. This reads directories, not filenames.

## Acceptance

- A five-level folder in the yardstick shape is read back as five levels, each
  correctly named.
- The rules it writes **parse**, **settle**, and **leave the folder they were
  read from exactly as it was** — the promise of "keep what I have".
- A *new* file arriving in `WhatsApp/` is filed to
  `2024/WhatsApp/Photo/jpg/under-100MB/`, or "maintain the shape" only means
  "do nothing".
- Files the shape does not explain are **left alone**, not swept.
- A directory named `2019` whose files are not from 2019 is read as a name.
- A crowded folder is offered a year split, with the numbers.
- By hand, against a real folder on disk, not only against a built snapshot.
