# Slice 7c tour: learning a shape a folder already has

`tungstate folder learn` reads a folder and writes down the shape it is already
in. This is the tour of why it is built the way it is, and of the two bugs that
only turned up by running it.

---

## 1. The dimension a file cannot tell you

The yardstick shape is `YEAR / APP / TYPE / EXTENSION / SIZE-GROUP`. Four of
those five are properties of the file: `{date:%Y}` from its date, `{ext}` from
its name, a `bucket` on `size`, and the type from `mime`.

**App is not.** Nothing inside `IMG-20240312-WA0001.jpg` says WhatsApp. It is
knowable only from where the file sits.

The obvious answer is the `parent` attribute, which is the file's immediate
directory. It does not work, and the way it fails is the interesting part. A
rule reading `path = "{date:%Y}/{parent}/Photo/{ext}/{band}"` would file
`WhatsApp/a.jpg` to `2024/WhatsApp/Photo/jpg/under-100MB/a.jpg` correctly — and
then on the next run that file's parent is `under-100MB`, so it would plan to
move it to `2024/under-100MB/Photo/jpg/under-100MB/`. And again the run after.
**A `{parent}` rule never settles**, which slice 6's convergence property would
catch and slice 7b's window refuses outright.

So the app has to be a **literal** in the path, and the rule has to be pinned to
the app some other way. `match.glob` runs against the file's root-relative
path, which is exactly the handle needed — with one wrinkle:

```toml
[[rule]]
name = "whatsapp-photo"
path  = "{date:%Y}/WhatsApp/Photo/{ext}/{band}"
match = { mime = "image/*", glob = ["WhatsApp/**", "**/WhatsApp/**"] }
```

Two arms, and the second is load-bearing. The first catches the file where it
is *now*, loose in `WhatsApp/`. The second catches it where this rule will
*put* it, at `2024/WhatsApp/...`. Without the second arm the rule stops
matching its own output, the file looks unmatched on the next run, and the
policy is unstable in the other direction.

Note what is **not** in the glob: the type. `Photo` is a literal in the path but
`mime = "image/*"` in the match, which is what lets a file arriving loose in
`WhatsApp/` be recognised as a photo without already sitting in a directory
called `Photo`. Names go in the glob because nothing else can know them; kinds
go in `mime` because something can.

### This was checked before it was built

The whole design rests on "a two-armed glob converges", and that is a claim
about the planner, not about taste. It was written by hand as a policy, applied
to a real folder, and re-planned:

```
=== SECOND plan (must be empty if it settles) ===
0 file(s) would move (0 B), 0 directories created, 0 removed, 6 left alone.
```

Twenty minutes, before a line of inference existed. Three of the last five
defects in this project came from building on an assumption that was never
executed.

---

## 2. Reading the shape back

`learn` takes a [`Snapshot`] and looks at one thing: the directory each file is
in, split into segments. For every file, at every depth, it asks what that
segment *looks like* — and then insists the file agrees:

- `2024` is a **year** only if that file's own date is 2024. Otherwise `2019` is
  somebody's project, and a test pins exactly that.
- `Photo` is a **kind** only if the file's media type is an image.
- `jpg` is an **extension** only if it is *this file's* extension.
- `under-100MB` is a **size band** by its spelling, which this tool chose in
  slice 5.
- Anything else is a **name** — an app, a client, a project.

A level is what four files in five agree it is; below that it is a name, on the
grounds that a level half of whose directories are years and half of which are
words is not an untidy date level, it is somebody's own vocabulary.

The shape is read from the depth **most** files sit at. A handful of files one
level shallower is a stray, not a second structure — and the command reports
what it covered rather than implying it covered everything:

```
5 of 18 file(s) sit in a shape 3 level(s) deep
```

That is the honest number for a half-organised folder, and it is deliberately
the first line.

---

## 3. Two bugs that only running it could find

### The cross product that invented directories

The first version asked each level for its values and wrote a rule for every
combination. Against a folder holding `Work/Clients/Acme` and
`Personal/Photos/Wedding` it produced:

```toml
path = "Personal/Personal/Personal"
```

Two name levels are a **tree**, not a product. `Work/Clients/Acme` and
`Personal/Photos/Wedding` are two branches; `Work/Photos/Acme` is not a third,
and no amount of squinting at the level lists reveals that — only the
combinations that actually occur do. It now enumerates observed branches, and
the regression test is named for the string it used to emit.

This is worth dwelling on because the unit tests at the time were green. They
were green on the *yardstick* shape, which has exactly one name level, and one
is the width at which a product and a tree are the same thing.

### The inbox that would have swept two thirds of the folder

`[folder].inbox` sends every file no rule matched to one directory. Every
canned layout sets one, so the generated policy set one too — and on the
chaotic folder the learned shape explains 5 files of 18, which would have meant
**13 files swept into `Unsorted/` by the policy whose entire promise is that it
keeps what you have.**

A learned policy now writes no inbox at all. Unmatched files are left exactly
where they are, and the count of unhomed files is reported as a *suggestion*,
where the person decides. "Keep the shape it already has" has to mean it.

---

## 4. Two policies, because a suggestion nobody can read is an opinion

`learn` returns the rules for the shape **as it stands** and, when it has
anything to say, the same rules **improved**. Both are printed; `--improved`
writes the second instead of the first.

Suggestions never arrive as taste. Each one carries the counts that justify it,
because "split by year" means nothing until you know what is in the folder.
Both of the following are real output — first, 412 photos in one directory
spanning five years:

```
what would be better:
  - Split by year.
    The fullest directory holds 412 files, and 412 of 412 have a date
    spanning 5 year(s) — about 82 per year.
```

And 34 photos filed one-per-directory, which is the opposite mistake:

```
  - Drop the `Photos` level.
    Every one of 34 files is under `Photos`, so that level is a directory
    everything shares and nothing is sorted by.
  - Drop the deepest level (extension).
    34 of 34 directories hold 2 file(s) or fewer — that level costs a click
    per file and groups nothing.
```

The thresholds are named constants with the reason in the docstring rather than
numbers inline (`CROWDED`, `THIN`, `LOPSIDED_PERCENT`, `AGREE_OF`), and they
are integer ratios rather than floats — these are ratios of counted files, so
integer arithmetic is both exact and what the surrounding code already does.

---

## 5. The probe policy, and a chicken-and-egg

A survey's cost comes from the policy it is given: `Policy::required_tier()` is
what decides whether tungstate reads no bytes, 8 KiB, or 64 KiB per file. But
learning happens *before* there is a policy — and it needs the media type (to
tell a `Photo` directory from a directory called Photo) and the date (to tell a
year from a project named `2019`), which are `head` and `meta` tier.

So there has to be *a* policy before there is one. `learn::PROBE` is it: a
constant in core, next to the code that needs it rather than copied into each
caller, with a test asserting it asks for `meta` **and nothing dearer**. That
test is the real point — it is what stops a later edit quietly turning every
`folder learn` into a whole-file hash of a video library.

---

## 6. What this does not do

- **It does not touch the window.** The layout picker should grow a fifth card —
  *keep the shape it already has* — and show both rule sets in the preview.
  That is 7d. The command line goes first because it is the reference surface,
  as it did for 5/5b and 7/7b.
- **It reads directories, not filenames.** `photo_2025-01-04.jpg` plainly
  carries a date, and `learn` ignores it and uses the file's timestamp. Reading
  dates out of names is a guess with a long tail of wrong answers, and the
  `regex` match key already exists for anyone who wants to write that rule
  themselves.
- **One shape per folder.** A folder organised two different ways in two
  branches gets the dominant one, and the `N of M` line is how you find out.
- **Suggestions are not applied to a written policy unless asked.** `--write`
  saves what you read; `--improved --write` saves the other one.

## 7. Verified

450 tests, fmt and clippy clean. Twelve of those are new in `learn`, and the
three that carry the weight are:

- `a_learned_policy_leaves_the_folder_it_was_learned_from_alone` — reads the
  shape, writes the rules, plans with them, asserts **no moves and settles**.
- `a_new_arrival_is_filed_into_the_shape_that_was_learned` — because a policy
  that merely does nothing would pass the one above.
- `files_the_shape_does_not_explain_are_left_where_they_are` — the inbox bug,
  pinned.

Plus one end-to-end CLI test against a real directory, which learns, writes,
re-plans to `0 file(s) would move`, and checks the second `--write` is refused
rather than overwriting somebody's rules.

By hand, on the two folders from slice 7b's walkthrough: the yardstick fixture
came back with the same five levels and essentially the policy that had been
written for it by hand, and the chaotic 18-file folder learned rules that plan
`0 file(s) would move, 19 left alone`.

**Not verified:** anything on screen — there is no screen yet. And the CLI
fixture had to be backdated to build, which is its own small proof: written
"now", a directory named `2024` is correctly read as a *name*, because its
files disagree with it.
