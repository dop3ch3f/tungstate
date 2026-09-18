# Slice 7d: filing defaults worth having, and trying them on

**Goal:** make choosing a shape a decision somebody can *see* rather than
guess at, and give them shapes worth choosing.

**Runnable outcome:** `tungstate folder compare ~/Downloads` shows what every
starting layout would do to that folder — including the rules it already has —
with real destinations, not descriptions. Three new layouts to choose from.
Nothing moves.

**This brief is the spec.** The tour is in `07d-tour.md`.

---

## Why

Slice 7b gave the window a layout picker: four cards with a sentence each. A
sentence is a description of a shape, and what somebody actually wants to know
is *what will this do to my files*. Those are different questions, and only the
second one has an answer they can check.

Worse, the picker only appears for a folder with **no rules**, so the moment a
folder is set up there is no way to ask "what would one of the others have
done?" at all.

The four layouts were also thin. `downloads`, `photos`, `documents` and
`by-type` between them offer one date shape and two flat ones, and none of them
resembles the five-level shape slice 7c proved out.

## Scope

**In:**

- Three new starting layouts:
  - **`media`** — `YEAR / APP / TYPE / EXTENSION / SIZE-BAND`, the five-level
    shape. The app comes from the name the app gave the file, per 7c.
  - **`by-date`** — year then month, the plainest shape there is.
  - **`by-source`** — one directory per app, nothing below it.
- `tungstate folder compare <PATH>` — every layout planned against the folder,
  side by side, with a real example move each, and the folder's current rules
  as the first row so everything else reads as a change from them.

**Out:**

- The window. The picker should offer this, and should offer it for folders
  that *already* have rules, which is 7e.
- Applying anything. `compare` changes nothing; `init --template` still picks.

## The thing that makes it cheap

The folder is walked **once** and every layout planned against that one
snapshot. Safe because `classify` applies each policy's own `ignore` when it
plans rather than only when it walks — so a shared walk cannot let one layout's
ignore list leak into another's answer.

412 files against 7 layouts in about a tenth of a second.

## Acceptance

- The three new layouts pass the existing property tests: they parse without
  warnings, describe themselves in under 46 characters, and **settle** when
  applied on paper.
- `media` files a WhatsApp photo, a Telegram photo, a screenshot and a plain
  camera photo each to the right five-level path, and settles on a folder it
  has already filed.
- `by-source` puts a WhatsApp photo under `WhatsApp` and not under `Camera`,
  which only rule order decides.
- `compare` lists every layout, names the folder's own rules first, and leaves
  the folder untouched.
