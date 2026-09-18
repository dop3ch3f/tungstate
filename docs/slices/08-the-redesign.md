# Slice 8: the redesign

The brief for rebuilding the window, written at the point the engine was locked
so it can be handed to a fresh session whole.

---

## The prompt

> Redesign the tungstate window.
>
> ### Start here, because it is new
>
> Two commands landed this week that change what the app is able to say.
> `learn_folder` reads the shape a folder is already in. `compare_folder`
> reports what every layout would do to that same folder, with counts and a
> real example move.
>
> Until now the app made people learn its vocabulary before it would show them
> anything. Add a folder, choose between four layouts described in sentences,
> and only then find out what happens to your files. That order can now be
> turned around. Somebody points at a folder, and the app shows them their own
> files, arranged the way they already keep them, beside what each alternative
> would do to them. Nobody has to know what a "layout" is to read that screen.
>
> Design for that. It is the interesting problem in this product and it has
> been out of reach until now.
>
> ### The feel to aim for
>
> CleanMyMac, for the shape of the interaction. Nothing asked up front, a
> result on screen, an obvious next move.
>
> Where that comparison stops working is worth keeping in view. CleanMyMac
> deletes things and has years of reputation behind it. Tungstate moves your
> files and has none, so the way back belongs on the same screen as the action
> rather than in a history tab. One button is right. One button with no visible
> undo is not.
>
> Keep four words: rules, preview, tidy up, put it back. They were argued over
> and tested on real screens, and they are not what is wrong. The shape of the
> interaction is being rebuilt, not the glossary.
>
> Everything else is open, including the dark palette. It was a deliberate
> choice for a tool that sits open during long transfers, and you may keep it,
> invert it, or throw it away.
>
> ### Where things are
>
> The window is about 3,700 lines of Vue 3, TypeScript and CSS in
> `crates/tungstate-gui/ui/`. That is what you are replacing, and it is the
> only thing you are changing.
>
> **Do not touch `crates/`.** Not a command, not a return type, not a comment.
> The engine underneath is about 21,500 lines of Rust across six crates with
> 467 tests, and it was deliberately finished and locked before this slice
> started so that the redesign would never need it. Every capability is already
> a command and every result already crosses as data.
>
> If you think you have found something the window cannot do without an engine
> change, you are probably wrong, and the way to check is `docs/SEAM.md`. If you
> are still sure after reading it, stop and say so. Do not write the Rust. A
> missing capability is a conversation about scope, not a quiet commit in the
> middle of a redesign.
>
> Read `docs/SEAM.md` first for what the window can ask for. Then
> `docs/slices/07b-tour.md` §7, which is what happened the last time nobody
> looked at the screens.
>
> ### Finding the look
>
> Follow the design method in `~/.claude/CLAUDE.md` under "Writing & Design",
> and read `~/.claude/references/ai-world-class-designer-guide.pdf` before you
> start. The `frontend-design` skill applies. The short version:
>
> Go broad first. You cannot be random on request, so bring the entropy from
> outside: generate a random string in a shell script and derive palette, type
> and layout from it, or work from one named and specific inspiration. Get at
> least three genuinely different directions up before you narrow. An idea that
> sounds bad on paper is worth one build.
>
> Then stop judging your own work. Screenshot the running app and hand the
> image, not the code and not your reasoning, to a fresh critic on a stronger
> model. Keep its prompt fixed: name the aesthetic, say how a good studio would
> execute it, list the biggest gaps, rank it against four real professional
> examples, score it out of ten. Watch whether the score is still moving after
> a round or two before spending more.
>
> Then take things away. Restraint is most of what separates this from
> template output, and the polish pass is mostly deletion. Prefer native
> components over custom ones, and real images, patterns or flat colour over
> gradients and glows built in code.
>
> For the copy, treat your first draft as lorem ipsum that shows the structure.
> Write it, then audit it against the tells in the guide, then rewrite. Run the
> `humanizer` skill on anything a user will read. Do as many rounds as it takes
> rather than trying to dodge the tells while drafting.
>
> ### Five things that have to stay true
>
> These are safety properties rather than taste, and a screenshot cannot reveal
> any of them. Full versions in `docs/SEAM.md`.
>
> 1. Nothing moves until the person has seen what would move.
> 2. Everything is reversible, and the way back is visible where it is needed.
>    `put_back` accepts any plan id, so any past reorganisation can be offered.
> 3. Files moved and operations performed are different numbers. Slice 7b
>    shipped "moved 9 file(s)" for five moved files.
> 4. "Left alone" has three causes: no rule claimed it, it is inside its
>    cooldown, or it changed while we looked. Collapsing those into one word
>    hides the only question worth asking.
> 5. Directories removed matters as much as files moved. It is the number that
>    shows a layout replacing a shape rather than adding to one.
>
> ### Then look at it
>
> Three of the last five screen defects in this project shipped green, and
> slice 7b found five more by opening the window that 435 passing tests had
> missed. Use the capture method in memory (`tungstate-screen-check-method`):
> one window, by its CoreGraphics id, scoped to the app's own pid. Never a
> region capture, which photographs whatever sits in front of the rectangle.
> Coordinates read off the preview need multiplying by 1.08.
>
> Every state gets looked at rather than inferred. Before calling it done:
> `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D
> warnings`, `cargo test --locked`, `npm run build`, and CI green on Linux,
> macOS and Windows.
>
> ### When to stop
>
> Stop when the critic scores eight or better twice running with no new gaps
> named, or when two rounds in a row move the score by less than a point. Say
> so rather than quietly polishing past it.
>
> Keep `docs/slices/08-tour.md` as you go: what you tried, what the critic
> said, what you threw away, and what you would still change.

---

## Three choices in the brief worth keeping

**It does not list the design tells to avoid.** The guide is explicit that
banning them up front makes a model overthink and invent stranger ones. Draft,
audit, rewrite.

**It names what must stay true and says nothing about how the app should
look.** The safety properties are the part a critic staring at a screenshot
cannot recover. No image shows that "left alone" has three causes.

**The critic never sees the code.** A builder scoring its own work is the
failure this project keeps running into wearing different clothes, most
recently 435 green tests and a window reporting that it had moved nine files
when it moved five.
