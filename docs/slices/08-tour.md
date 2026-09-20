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

## What is still not verified

- The script has only been run on macOS. `touch -t`, `dd ... count=0 seek=`
  and sparse files all behave differently enough elsewhere that Linux is a
  guess until someone runs it.
- Nothing has been screenshotted yet.
