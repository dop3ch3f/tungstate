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

### The one that was kept

**C, the workbench.** The deciding difference was not palette: it was that C
shows all eight ways of filing at once and the other two show three. On a
screen whose entire job is comparison, making someone scroll to see half the
options is not a small cost. The bar on each tile is the other half of the
argument — it answers "how much of my folder does this touch" before any number
is read, which is exactly the question somebody who does not know what a layout
is would ask first.

A and B are deleted rather than parked. Keeping a losing direction around is
how a codebase ends up with two half-finished design systems.

What A and B were better at, which the next rounds should try to recover:
A's figures were genuinely comparable down a column, because they were tabular
and aligned; C's are inside tiles and cannot be scanned the same way. B was
the most restrained of the three, and restraint is the thing this project keeps
having to add back later.

### What is deliberately not in them

Each direction is one screen, not an app. There is no preview, no tidy, no put
it back, no drain half. Building three whole applications to choose one is the
version of going broad that cannot be afforded, and the composition question —
how does a folder and eight possible futures for it fit on one screen — is
answerable from this screen alone.

---

## 3. Six rounds with a critic that never saw the code

The method the brief points at is specific about why a builder cannot judge its
own work: it reviews its own decisions and its own reasoning along with the
pixels. So the critic here got a screenshot and nothing else. Fresh context
every round, a stronger model, the same prompt every time, and no sight of the
code, the repository, or its own previous answers.

The prompt is fixed and lives in the tour rather than in the repo, because it
is method rather than product: name the aesthetic, say how a top studio would
execute it, list the biggest gaps element by element, rank the screenshot
against four named professional applications, score it out of ten.

### The scores, and the honest reading of them

| Round | Score | What changed going in |
|---|---|---|
| 1 | 5 | the first build: eight tiles in a grid |
| 2 | 6 | a list with aligned columns; the grid deleted |
| 3 | 5.5 | bars deleted, one selection state, a measure |
| 4 | 5.5 | one accent, no callout, constant row height |
| 5 | 6 | figures on the row's baseline, plain column names |
| 6 | 6 | one left edge, inset footer, hollow middle closed |

**It never reached eight, and it ranked the screen fifth of five in every
round**, behind Things 3, Linear, iA Writer and Arc. The brief says to say so
rather than quietly polishing past it, so: this is a six, by a critic that
scores hard and was asked to rank against four of the best-made applications
on the platform.

The stopping rule fired twice over. Rounds 3 to 4 moved the score by zero and
rounds 5 to 6 moved it by zero, and the brief stops at two consecutive rounds
moving less than a point. What is still on the critic's list at the end is
mostly things it has said before and that were answered deliberately rather
than missed: it wants the primary button to carry an accent, and the brand
reserves that colour for what is live or chosen; it wants fewer text styles
than a table with a heading, a caption and column labels can honestly have.

### What the critic was worth

Three findings paid for the whole exercise, and none of them were about taste.

**It caught a real inconsistency in the numbers.** Round 1, first item: the
sidebar said "28 of 30 files" and every row said "of 31". That is not a
rendering slip. `learn_folder` counts files the probe policy did not ignore;
`compare_folder` counts every entry it walked. Same folder, same moment, two
denominators, and the redesigned screen is the first surface in this project
that would ever have shown them side by side. The window now states one total
and only one.

**It named the grid as a dashboard.** The first build drew eight tiles each
with its own bar. Eight bars in eight boxes share no axis, so the one thing
the screen exists to do — compare — was the one thing it could not do. That
charge turned the screen into a list and is the single largest change across
all six rounds.

**It kept finding the same fault under different names.** Too many accents,
said three times in different words. Wrapped headers, said twice. A note made
twice by a critic that cannot remember its last answer is worth much more than
a note made once, and that rule decided what got acted on.

### The instrument was broken for four rounds

Round 4 complained the proportions were "off by roughly 1.5x throughout": the
body copy at 22-24px, rows 108px tall, a title larger than any Mac app needs.
None of that was true. The body is 14.5px and the rows are 54.

`screencapture` returns the window's backing store, which on this display is
2x. The critic was reading device pixels as points and marking the design down
for being twice the size it is. Three of round 4's eleven findings came from
that and were worthless.

The fix is one line — `sips -z 720 1080` before handing the image over — and
the lesson is larger than the fix: **a critic that measures needs to be given
the units it thinks it is measuring in.** Rounds 5 and 6 were run on
correctly-scaled images, which is why their complaints turned concrete: a
button five pixels from the window edge, two left margins 12px apart, a rule
that stops where the selection band does not.

### Three defects only looking found, again

- **A sentence that was there and could not be read.** The note explaining why
  the folder's own rules are not offered rendered at the dimmest grey in the
  palette at 11.5px. It was in the DOM, it was in the screenshot, and it was
  invisible.
- **Then the same sentence hidden behind the furniture.** Made readable, it
  moved below the fold, where the sticky footer drew straight over it. The
  content overflowed by about thirty pixels and the thing it hid was the one
  sentence on the screen that explains a refusal.
- **A grid that was not a grid.** Every row is a `<button>`, and a button
  shrinks to fit. Seven rows meant seven grids of seven different widths, each
  landing its figures somewhere else, under headers that belonged to none of
  them. `width: 100%` fixed it. Before that, `1fr` had already had to become
  `minmax(0, 1fr)`, because a bare `1fr` has an `auto` minimum and the longest
  description was widening its own row alone.

None of the three is visible in code review. All three are obvious in a
screenshot, which is the argument this project keeps re-learning.

---

## 4. A script that reads the CSS nobody reads

Slice 7b opened the window and found eleven defects. Four of them were one
thing: **a selector that matches nothing.** `.mono` used three times and
defined nowhere. `.path` scoped to `.ledger .path`, so the two paths written
in prose lost it. `.spacer` defined only under `.topbar`, so the toolbar it
was written for matched nothing and its buttons never moved right.
`aria-current` bound in three places with no rule to draw it.

Every one renders as *slightly wrong* rather than as an error. Nothing fails,
so nothing tells you. Building the three directions produced three more of the
same family before a single real screen existed.

There is no off-the-shelf fix inside the constraint of not adding a
heavyweight dependency. Stylelint needs a plugin and a config and still would
not cross the template-to-stylesheet boundary. CSS Modules do not help either:
`$style.typo` is `undefined` at runtime and typed `string` by Vue, so it fails
silently in exactly the same way.

So: `ui/scripts/check-css.mjs`. Node, no dependencies, about 200 lines, and
wired into `npm run build` ahead of `vue-tsc`. CI already runs `npm run build`
on Linux, macOS and Windows, so this guards all three with no workflow change.

Six rules, each traceable to something that shipped:

| Rule | The defect it would have caught |
|---|---|
| `unstyled-class` | `.mono`, used three times, defined nowhere |
| `orphan-selector` | `.conns .addr.root`, a dead token nothing matched |
| `undrawn-state` | `aria-current` on the open folder and the active view, neither drawn |
| `undeclared-token` | a `var(--x)` that falls back to nothing, silently |
| `raw-token-leak` | a component reaching past the semantic layer into the palette |
| `template-literal-class` | a backtick class binding, which cannot be checked at all |

It was tested against a file written to break all six before it was committed,
because a checker that is itself wrong reddens three platforms at once. It
catches all six.

### The bug it had, which is the bug it exists to catch

The first version read everything between `<template>` and the **first**
`</template>`. Vue templates contain nested `<template v-if>` elements, so it
stopped a third of the way down the file and reported the rest as clean. Worse,
the same assumption had already been written into the script that prefixed the
directions' class names, which is why three classes in one file were renamed in
the stylesheet and not in the markup, and `.a-fig.sub` quietly matched nothing.

The same wrong idea, written twice, producing the exact defect the tool was
being built to find. It reads to the **last** `</template>` now.

### What it cannot do, said plainly

It proves existence, not correctness. It cannot see that a row is three pixels
too tall, that a colour is wrong, or that the layout breaks at 860px. Those are
what §3 and the state walk are for. It also cannot follow a computed class
name, which is why a backtick binding is an error rather than a warning: a
lookup object reads better anyway.

### The tokens underneath it

`src/styles/tokens.css` is the only file in the project with a literal colour,
size, duration or radius, and the checker enforces it. It is two layers: a raw
palette named for what the colours are, and a semantic layer naming what they
mean. Nothing outside the file may reach for a raw token. Re-tinting the whole
application is editing the first block; re-deciding what a colour *means* is
editing the second.

`base.css` holds the reset and three classes. A class earns a place there only
if it says *what a thing is* rather than where it sits: `.path`, `.mono`,
`.num`. That is slice 7b's generalisation kept rather than relearned.

---

## 5. Making "moved 9 files" impossible to write

The worst defect slice 7b found was not a styling slip. Tidying a folder
promised "5 of 6 files would move" and then reported **"moved 9 file(s)"**.
`Applied::done` counts *operations*, and that plan also made three directories
and removed one: 5 + 3 + 1 = 9. Somebody who read both numbers was told their
folder had been rearranged nearly twice as much as it was, and it survived 435
passing tests because nothing tested the sentence.

`docs/SEAM.md` made it safety property 3. `src/lib/counts.ts` makes it a
compile error.

```ts
declare const isFileCount: unique symbol;
export type FileCount = number & { readonly [isFileCount]: true };
const seal = (n: number) => n as FileCount;

export const moved = (done: TidyDone): FileCount => seal(done.moved);
export const wouldMove = (view: PreviewView): FileCount => seal(view.files);
```

The trick is what is **not** there: no `asFileCount(n: number)`. A `FileCount`
cannot be made from an arbitrary number at all. The only way to get one is to
hand over the engine value that genuinely is a count of files, and every one of
those has a named accessor. Everything else — `moves.length`, `ops.length`, a
sum of files and directories — is a plain `number` and structurally not
assignable.

**Rust note for the tour:** this is the same idea as a newtype. `struct
FileCount(usize)` in Rust makes a count of files a different type from a count
of operations, so the compiler refuses to confuse them even though both are
`usize` underneath. TypeScript has no newtypes, so the same effect is faked by
intersecting `number` with a property keyed by a symbol nobody else can name.
It is a phantom: `seal` compiles to nothing and the value at runtime is just a
number. The whole mechanism exists at type-check time and then evaporates.

Proved, rather than assumed:

```
src/proof.ts(6,28): error TS2345: Argument of type 'number' is not
  assignable to parameter of type 'FileCount'.
```

That was `files(view.moves.length)` — the bug, written on purpose. The raw
field `done.moved` is refused too; only `moved(done)` passes. `vue-tsc` runs
inside `npm run build`, which CI runs on Linux, macOS and Windows.

Directories get the same treatment for the opposite reason. `DirCount` is a
separate type because safety property 5 says directories removed is the number
that says a layout is *replacing* a shape rather than adding to one. Keeping
them un-addable to a `FileCount` is how the two numbers stay two numbers.

### The rest of the layer

`engine/types.ts` is data only — no Vue, no Tauri — so a screen that needs to
know what a `PreviewView` is does not pull `@tauri-apps/api` into its module
graph to find out. The old `api.ts` mixed fifty `invoke` wrappers with
`bytes()`, `kind()` and `mark()`, which is why every component imported `api`
even when it only wanted to format a number.

`engine/commands.ts` holds the calls, grouped by the question being asked the
way `SEAM.md` groups them. `engine/events.ts` holds the twelve
`transfer://` listeners, with a note at the top that tidying emits none of
them, so nobody reaches for a progress bar that cannot exist.

`lib/tone.ts` closes a live bug. `TransfersView.vue:218` does
`:class="row.state"` and `ActivityView.vue` does `:class="op.status"`: an
engine string put straight into the DOM. Add an outcome on the Rust side and
the row renders with no rule at all — unstyled, not broken, nothing fails. That
is the same family as the four selectors matching nothing, arriving from the
other direction. Every such map is now total, with an explicit fallback.

---

## What is still not verified

- The script has only been run on macOS. `touch -t`, `dd ... count=0 seek=`
  and sparse files all behave differently enough elsewhere that Linux is a
  guess until someone runs it.
- Only one screen exists. There is no preview, no tidy, no put it back, and
  no drain half yet, and the comparison screen is drawn from captured data
  rather than from a live call.
- 1080x720 only. The 860x560 minimum has not been looked at once.
- macOS only. Nothing has been seen on Linux or Windows.
- The critic scored a six, not an eight. The stopping rule was the
  plateau clause, not the quality clause.
