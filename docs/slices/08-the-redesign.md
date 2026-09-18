# Slice 8: the redesign

The brief for rebuilding the window. Written at the point the engine was
locked, so that it can be handed to a fresh session whole.

---

## The prompt

> Redesign the tungstate window.
>
> **What this is.** Tungstate keeps folders in the shape you declared, and
> moves files between machines without losing them. It is a Tauri app: a Rust
> engine of about 21,500 lines across six crates, and a window of about 3,700
> lines of Vue 3, TypeScript and CSS in `crates/tungstate-gui/ui/`. You are
> replacing the second number.
>
> **Read first, in this order:** `docs/SEAM.md` — the contract between engine
> and window, and the list of safety properties a redesign has to keep.
> `docs/slices/07b-tour.md` §7 — what happened the last time nobody looked at
> the screens. `docs/DESIGN.md` §1 for what the product promises.
>
> **The hard constraint.** This needs no Rust. Every capability is already a
> command and every result already crosses as data. If you find yourself
> needing an engine change, stop: that is a hole in the seam, and it belongs in
> `docs/SEAM.md` and a separate commit, not quietly inside the redesign.
>
> ### What is wrong with the window today
>
> It asks you to learn its vocabulary before it shows you anything. You add a
> folder, then choose between layouts described in sentences, and only then see
> what any of it would do to your files. The layout picker appears only for a
> folder with no rules, so once a folder is set up there is no way to ask what
> a different shape would have done.
>
> Two commands added recently change what is possible: `learn_folder` reads the
> shape a folder is already in, and `compare_folder` reports what every layout
> would do to it, with counts and a real example move. The window can now show
> somebody their own folder before asking them to decide anything.
>
> ### The brief
>
> Point at a folder, see what we found, press one button. Take the shape of the
> interaction from CleanMyMac: ask nothing up front, show a result, make the
> action obvious. Note where the comparison breaks — CleanMyMac deletes things
> and is trusted by reputation; this moves your files and has no reputation
> yet, so the way back has to be visible at the moment it is needed rather than
> filed in a history screen.
>
> Keep the plain-language vocabulary: rules, preview, tidy up, put it back.
> Those words were argued over and tested on the existing screens. The
> interaction is what is being rebuilt, not the glossary.
>
> The visual direction is open, and the guide's method for finding one is
> below. Dark is the existing design and is deliberate, but it is not a
> requirement.
>
> ### What must survive, whatever it looks like
>
> From `docs/SEAM.md`, and these are safety properties rather than taste. A
> simpler screen makes them matter more, not less.
>
> 1. Nothing moves without the person having seen what would move.
> 2. Everything is reversible, and the way back is visible where it is needed.
>    `put_back` takes any plan id, so any past reorganisation can be offered.
> 3. A count of files and a count of operations are different numbers. Slice 7b
>    shipped "moved 9 file(s)" for five moved files.
> 4. "Left alone" has three different causes — no rule claimed it, it is inside
>    its cooldown, or it changed while we looked. A screen that collapses those
>    into one word hides the only question worth asking.
> 5. Directories removed matters as much as files moved. It is the number that
>    says a layout is replacing a shape rather than adding to one.
>
> ### How to work
>
> Follow the design method in `~/.claude/CLAUDE.md` ("Writing & Design"), and
> read `~/.claude/references/ai-world-class-designer-guide.pdf` before starting.
> The `frontend-design` skill is installed and applies. In short:
>
> **Discover.** Go broad before deep. You cannot be random on request, so bring
> entropy from outside: generate a random string with a shell script and derive
> palette, type and layout from it, or work from a named, specific inspiration.
> Produce at least three genuinely different directions before narrowing. Ideas
> that sound bad are worth one try.
>
> **Define.** Do not judge your own work. Screenshot the running app and hand
> the image — not the code, not this conversation — to a fresh-context critic
> on a stronger model. Keep the critic's prompt fixed: name the aesthetic, say
> how a top studio would execute it, list the biggest gaps, rank it against
> four real professional examples, score out of ten. Check whether the score is
> still moving after one or two rounds before spending more.
>
> **Deliver.** Restraint reads premium, and the polish pass is mostly deletion.
> Prefer native components over custom ones. Prefer real images, patterns or
> solid colour over gradients and glows built in code.
>
> **Copy.** Treat your first draft as lorem ipsum: it shows the structure, then
> every line gets rewritten in one plain voice. Draft it, then audit against
> the tells listed in the guide, then rewrite. Do that as many rounds as it
> takes rather than trying to avoid the tells while writing.
>
> ### Verification, which is not optional here
>
> Three of the last five screen defects in this project shipped green, and
> slice 7b found five more by opening the window that 435 passing tests had
> not. Use the capture method recorded in memory
> (`tungstate-screen-check-method`): one window, by its CoreGraphics id,
> scoped to the app's own pid. Never a region capture — it photographs whatever
> is in front of the rectangle. Remember the preview is scaled: multiply
> coordinates read off it by 1.08.
>
> Every state gets looked at, not inferred. Before declaring it done:
> `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D
> warnings`, `cargo test --locked`, `npm run build`, and CI green on Linux,
> macOS and Windows.
>
> ### Stopping criteria
>
> Stop and show me when the critic scores eight or higher twice running with no
> new gaps named, or when two consecutive rounds move the score by less than a
> point. Do not keep polishing past that without saying so.
>
> Write `docs/slices/08-tour.md` as you go: what you tried, what the critic
> said, what you threw away, and what you would still change.

---

## Why the brief is shaped this way

Three things in it are deliberate and easy to undo by accident.

**It does not list the design tells to avoid.** The guide is explicit that
banning them up front makes a model overthink and invent stranger ones. The
instruction is draft, audit, rewrite.

**It names what must survive, and nothing about how it should look.** The
safety properties are the part that cannot be rediscovered by a critic looking
at a screenshot: no image shows that "left alone" has three causes.

**It sends the screenshot to a critic that cannot see the code.** A builder
scoring its own work is the failure this whole project keeps meeting in other
costumes — 435 green tests and a window telling somebody it had moved nine of
their files.
