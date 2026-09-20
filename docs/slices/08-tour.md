# Slice 8 tour: the redesign

The brief is [08-the-redesign.md](08-the-redesign.md). It asks for the window to
be rebuilt around two commands the window has never called, and for the engine
not to be touched while it happens.

This tour is written as the slice runs rather than after it, because most of
what is worth recording is what got thrown away, and that is only legible at
the time.

---

## 1. A folder worth pointing the window at

Nothing in this repo has ever built test data. Every earlier slice made its
folders inside a `tempfile` and threw them away when the test ended, which is
right for a test and useless for looking at a screen. Slice 7b's screen check
built its folders by hand, twice, and the second set did not match the first.

So this slice starts with `crates/tungstate-gui/ui/scripts/demo-folder.sh`,
which builds ten folders under `$TMPDIR`, one per condition the window has to
be able to draw.

It lives in `ui/scripts/` rather than anywhere more obvious for three reasons
that all had to hold at once: it is inside the only directory this slice is
allowed to change; `index.html` never references it, so Vite cannot pull it
into a bundle; and it is not in `ui/public/`, which is copied verbatim into
`dist/` and would have shipped it to users.

### Three things about it that are not obvious

**Magic bytes, or the folder exercises nothing.** Mime is sniffed from a file's
first bytes and only falls back to the extension for text formats
(`tungstate-attrs/src/lib.rs:195`). `.jpg` is not in that fallback table. So an
empty file called `holiday.jpg` is `application/octet-stream`, matches none of
the image rules in any layout, and a demo folder built from `touch` would have
produced a comparison screen where every layout moves nothing — a screen that
looks plausible and tells you nothing. Every file the script makes starts with
real magic bytes for its kind.

**Holes, so a laptop can hold 18 GB.** The media layout files by size band, and
the top band is `>1GB`. Writing those for real is absurd for a fixture, so the
script writes the header and then extends the file without writing:

```sh
dd if=/dev/zero of="$path" bs=1 count=0 seek="$size"
```

`count=0` means no blocks are copied; `seek` past the end just moves the file's
size. The engine reads size from metadata, so a hole is as big as it says it
is. The fixture is 18.4 GB apparent and 601 MB on disk, and almost all of the
601 MB is the three files that are deliberately real.

**The two tidy folders are built by the engine, not by hand.** `already-tidy/`
has to match `downloads.toml` exactly, and `real-shape/` has to be in a shape
the media layout would have produced. Writing those directory names out by hand
is guessing, and a fixture that guesses wrong sends you hunting for a bug in
the window that is really a bug in the fixture. Instead the script makes the
folders messy, then runs the real CLI over them:

```sh
"$CLI" init "$T" --template downloads
"$CLI" apply "$T" --yes
```

For `real-shape/` it then deletes `.tungstate/` again, which leaves a folder
that is genuinely in a five-level shape and genuinely ungoverned. That is
precisely what `learn_folder` exists to read, and it is not a shape anyone
typed.

It works. The engine files them as:

```
2026/WhatsApp/Photo/jpg/under-100MB/IMG-20260425-WA006.jpg
2026/Camera/Photo/jpg/over-1GB/DSC020265.jpg
```

which is the five-level shape from `docs/DESIGN.md` with the band names
spelled the way Windows will accept — `under-100MB`, not `<100MB`.

### What each root is for

Ten roots, each unlocking states that are otherwise unreachable:

| Root | What it makes reachable |
|---|---|
| `real-shape/` | `learn_folder` finding five levels and 192 of 192 files, with no suggestions |
| `messy-downloads/` | one level, 2 of 30, 28 loose, several suggestions, so `improved` is not null |
| `photos-by-nothing/` | no shape at all, 120 files loose |
| `already-tidy/` | `tidy: true` and `already_tidy: true`, meaning it |
| `has-rules/` | the `(the rules you have)` row, and the `give_rules` refusal |
| `broken-rules/` | `loads: false`, with a line and column |
| `unsettled/` | `settles: false`, the one refusal the window keeps |
| `huge/` | 5,000 files, for the tree and for timing a tidy that cannot report progress |
| `empty/` | every count zero |
| `denied/` | a directory that cannot be read |

`messy-downloads/` also carries the cases that only bite in person: a symlink, a
directory named `archive.zip`, a file with no extension, a name with an emoji,
and a file written at generation time so it is inside its cooldown and reads as
*too recent* rather than as *nothing claimed it*.

Running `folder compare` against it gives the comparison screen its material:

```
downloads    28 of 31 would move, 6 dir(s) made, 1 removed
by-source     1 of 31 would move, 1 dir(s) made, 0 removed
photos       10 of 31 would move, 6 dir(s) made, 1 removed
```

Three layouts, three genuinely different answers, and the one number that says
a layout is replacing a shape rather than adding to one is already different
between them.

### What it will not fake

Two states need a process to die or a conflict to happen, and producing them by
writing journal rows would mean reaching around the engine, which is the one
thing this slice must not do. The script prints them as recipes instead:

- **an interrupted run** — start a transfer of `to-drain/`, `kill -9` the
  process part-way, reopen the window;
- **a quarantine** — run a link with `on_conflict = "quarantine"` against
  `nas/incoming`, which already holds a different `talk.mp4`.

### The bug it had, which is the bug this kind of script always has

The first run failed on `touch: out of range or illegal time specification`.
A counter used to build filenames was also being used to build a timestamp:

```sh
i=$((i + 1))
mkf "$D/$name.jpg" $(...) "2026060${i}0930"
```

`i` was still counting from the previous loop, so by the sixth file the stamp
was `2026060110930` — thirteen digits where `touch` wants twelve. It failed
loudly, which is the only reason it took two minutes rather than a week. The
fix is a second counter that resets per loop. Worth recording because the
interesting version of this bug is the one where `i` happens to stay single
digit and the fixture silently backdates half its files to the wrong day.

---

## 2. Three directions, and where they came from

The brief asks for at least three genuinely different directions before
narrowing, and is specific about why asking a model for variety does not
produce any: asked to be unique it returns the most probable version of
unique. The variety has to come from outside.

So `scripts/seed.sh` prints a random string, and a mapping recorded here turns
it into design decisions. The mapping was written before the seeds were drawn:

| Property | Derived from |
|---|---|
| base hue | sum of character codes, mod 360 |
| ground | count of digits, mod 4 → near-black / mid-grey / paper-white / ink-on-cream |
| density | count of capitals, mod 3 → tight / normal / airy |
| type pairing | longest run of letters, mod 4 |
| radius | squared if the string contains `/` or `+`, else rounded |
| where the accent is spent | sum of character codes, mod 4 |

The three seeds, and what they gave:

```
A  O78kyuj9pqQf6spnR7sVw7PYaR3AXpIX
   ink-on-cream · airy · serif text + grotesque figures · rounded
   accent on the thing you have chosen

B  VzSvLYkYxGnz7wODb4pMDdMUbJnUsLXh
   paper-white, warm · normal · mono throughout · rounded
   accent on what is live now

C  xYxWfgJunLJWLZWKARK0febtYtFqFxpT
   mid-grey, warm-green · airy · humanist + mono · rounded
   accent on what would change
```

**One intervention, recorded rather than hidden.** The first three seeds all
landed on a light ground, because two of the four ground values are light and
three draws went that way. That is a real result and not a bad one, but it
would have left nothing dark to compare against, and the dark palette was a
deliberate choice for a tool that sits open during long transfers. So C's seed
was redrawn until the ground came out dark. It took one draw. Nothing else was
redrawn, and the mapping was not adjusted after the fact.

The seed sets palette, type and density. It does not set composition, and
composition is where the three actually differ:

- **A, the measuring instrument.** One column, one row per way of filing, the
  figures set large and tabular so the eye compares down the column rather than
  reading each row.
- **B, the report.** A printed report of the kind a line printer used to
  produce. Monospaced throughout, column-aligned, ruled hairlines, no cards and
  no containers.
- **C, the workbench.** A work surface. Each way of filing is a tile carrying a
  bar of the folder itself: the blue part is what that layout would move, the
  grey part is what it would leave.

### They are fed real answers, not invented ones

`src/looks/captured.ts` holds output from `tungstate plan --json` and
`tungstate folder learn` run against `messy-downloads/`. Every number, name,
path and sentence is the engine's.

This is not fussiness. A mockup fed invented data gets comfortable string
lengths and tidy two-digit numbers, and then the real screen arrives with
`Screenshot 2026-09-01 at 11.02.44.png → Images/Screenshot 2026-09-01 at
11.02.44.png` and has nowhere to put it. All three directions had to deal with
that path, and they deal with it differently, which is itself part of what is
being judged.

The two rows a mockup would never invent are in there too: the
`(the rules you have)` row with an empty summary, and the same row with
`settles: false`, which is the one refusal the window keeps.

### What building them found

Three defects, all the same family, all invisible without opening the window.

**A global class name reached into a component that never asked for it.** The
first direction used `class="sheet"` and `class="lede"` for its own layout.
Both already exist in `styles.css`, where `.lede` sets `color: var(--light)`.
On a cream ground that is near-white on near-white: the folder name and the
opening paragraph rendered as faint smudges. Nothing failed. The fix was to
prefix every class in each direction — `a-`, `b-`, `c-` — so the stylesheet
being replaced cannot reach in while it is still loaded.

**A selector that matched nothing, again.** `.a-fig.sub` in the stylesheet
against `class="a-fig a-sub"` in the template, so the two smaller figures lost
their stacking and rendered as `6made`. This is the same defect slice 7b found
eleven of, and it took about four minutes to find *because a script was
looking*, rather than an afternoon of squinting.

That script is the interesting part. It reads each component, collects the
classes its template uses and the classes its styles define, and prints the
difference both ways. It found five more misses immediately. It also had a bug
worth keeping: it took everything between `<template>` and the first
`</template>`, and Vue templates contain nested `<template v-if>` elements, so
it silently stopped reading a third of the way down the file and reported the
rest as clean. The same assumption is why the prefixing pass missed those
classes in the first place. Both now use the *last* `</template>`, and the real
checker in the next commit inherits the lesson.

**A blanket rule outranked the rule it was protecting.** Forcing colour with
`.c-bench button { color: inherit }` beats `.c-go { color: ... }`, because a
class plus a type selector outranks a class alone. The primary button lost its
dark text and turned pale blue on pale blue. Restating it as
`.c-bench .c-go` fixed it. Specificity is not a style question here: the button
was unreadable.

### What is deliberately not in them

Each direction is one screen, not an app. There is no preview, no tidy, no put
it back, no drain half. Building three whole applications to choose one is the
version of going broad that cannot be afforded, and the composition question —
how does a folder and eight possible futures for it fit on one screen — is
answerable from this screen alone.

---

## What is still not verified

- The script has only been run on macOS. `touch -t`, `dd ... count=0 seek=`
  and sparse files all behave differently enough elsewhere that Linux is a
  guess until someone runs it.
- The three directions have been photographed at 1080x720 only. Neither the
  860x560 minimum nor any state other than this one screen has been seen.
- Nothing outside the comparison screen has been designed at all yet.
