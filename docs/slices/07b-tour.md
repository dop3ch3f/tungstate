# Tour: slice 7b

The brief is in [07b-the-window-tidies.md](07b-the-window-tidies.md).

A migration, four TOML files, and a screen. The Rust is quiet; what is worth
slowing down for is a set of decisions about **what a person is looking at**,
because this is the first slice whose success or failure is measured in whether
somebody understands the thing in front of them.

---

## 0. The glossary, since there are now two vocabularies

| On screen | In the code, the CLI and the docs |
|---|---|
| Rules | policy |
| Preview | plan |
| Tidy up | apply |
| Put it back | undo |
| Left alone | untouched / `Reason` |
| Set aside | quarantine |
| Now / After tidying | the snapshot before and after |

This is a real cost and worth naming before anything else: two words for one
idea means the docs no longer read straight onto the screen. It is paid because
the alternative is worse. Somebody opening this app has a messy Downloads
folder; they do not have a mental model of a reconciliation controller, and
*policy, plan, apply, drift* is four pieces of jargon before the first useful
action — four chances to close the window.

The command line keeps every precise word, because the command line is where
you go when you need to be precise.

---

## 1. The wall this slice exists to remove

Before this, the second half of tungstate was reachable only by somebody
willing to hand-write TOML. That is not a small barrier, it is *the* barrier:
everything the planner and the executor can do was sitting behind a file format.

Four starting layouts now stand where the wall was, compiled straight into the
binary:

```rust
body: include_str!("../../../policies/downloads.toml"),
```

`include_str!` reads a file **at compile time** and substitutes its contents as
a `&'static str`. The layouts are therefore not data files to be found, shipped
or lost — they are part of the executable, and a build that cannot find one does
not compile.

### They are held to a higher bar than a policy you wrote

Because they are somebody's first impression:

```rust
#[test]
fn every_starter_layout_settles_on_a_messy_folder() {
```

Every layout must parse with **no warnings**, describe itself in a line short
enough for a picker, and **settle** — plan it, apply it on paper, plan again,
and there is nothing left. The worst possible first impression is a layout that
tidies your folder and then wants to tidy it again for ever.

That test earned its place immediately: it caught the same syntax error in three
of the four layouts before anybody saw them. I had written

```toml
vars.year = { from = "mtime", format = "%Y" }
```

when the format belongs in the *placeholder*, not the variable:

```toml
path = "{date:%Y}/{date:%m}"
vars.date = { from = ["exif.DateTimeOriginal", "mtime"] }
```

Three broken files, caught by a property nobody had to think to check.

### Creating is not editing

Slice 5b decided the window never edits a policy: the file lives in the folder,
is meant to be committed, and a form cannot represent everything the parser
accepts. That still holds, and the layouts do not bend it. Writing a *first
draft* is scaffolding; the window shows the file read-only afterwards and says
where it is. `init` refuses to overwrite one that exists, because a policy is
somebody's work and this feature exists to get them started, not to start them
over.

---

## 2. A registry, and the deferral it retires

Slice 6 refused to build a `folders` table, and the reason was written down:

> A registry is a list, and a list needs an iterator to be worth having.

The only thing that iterated folders was the daemon, three slices away. That
was right, and **this slice is what retires it**: a window is an iterator.

`tungstate plan` finds its folder by walking up from wherever it was typed,
which is exactly right for a command you run *inside* a folder. A window has no
working directory to walk up from. It has to be able to show you the folders you
have, which means something has to remember them.

Two small decisions in the schema:

```sql
root     TEXT NOT NULL UNIQUE,
name     TEXT NOT NULL,
```

**The root is the identity**, so adding the same folder twice is one folder
rather than two rows disagreeing about its name. **The name is not unique**,
because two different `Photos` directories on two drives are a perfectly
ordinary thing to have and a name is only for reading.

And forgetting a folder keeps everything:

> Forgetting a folder is forgetting to *watch* it, not forgetting what was done
> to it.

so `log` and `undo --plan` keep working on a folder that is no longer governed.

---

## 3. Two trees, and why they were nearly free

The obvious preview is a list of `from → to` lines — it is exact, and it is what
the command line already prints. It was rejected as the *default* because it
asks the reader to assemble the answer themselves. The question somebody
actually has is **what will my folder look like**.

The reason this was cheap is the interesting part:

```rust
let after = plan.apply_to(&before)?;
```

Slice 6 built `apply_to` as a paper model, to check that a plan is executable
and that the rules settle. Slice 7 reused it as the executor's specification.
Here it becomes a *picture* — the "after" tree is a value the engine was
computing anyway, for reasons that had nothing to do with drawing it.

That is the third job one function has done in three slices, and it is the
strongest argument in the project for keeping the engine's outputs as plain data
rather than as side effects.

Both trees mark the same files:

```rust
let moving_from: BTreeSet<&str> = plan.ops.iter().filter_map(Op::source).collect();
let moving_to:   BTreeSet<&str> = plan.ops.iter().filter_map(Op::target).collect();
```

so a file is highlighted where it is *and* where it would be, and the eye can
follow one across. The interface has exactly one accent colour, and this is what
it is spent on — the only thing anybody is looking for on this screen.

---

## 4. Three places the screen refuses to be vague

**A folder with no rules.** It does nothing at all, and that is not guessable
from anywhere else. So the list says `none yet` in the warning colour, and
opening it shows the picker rather than an empty preview.

**Rules that will not load.** Loaded when the *list* is built, not when the
folder is opened:

```rust
broken: has_rules.then(|| govern::rules_at(&root).err()).flatten(),
```

so a broken policy is visible in the table rather than only after clicking in.
The window has no `miette`, so the line number goes into the sentence instead of
being drawn under the source:

```rust
let (line, column) = tungstate_core::line_of(text, span.start);
format!("line {line}, column {column}: {error}")
```

Same `line_of` the command line uses — slice 5 put it in core precisely so the
two surfaces could not disagree about which line something is on.

**A folder that is not tidy *yet*.** This one was a bug on the command line
first, found by running it:

```
$ tungstate apply /tmp/tw8
nothing to do: `downloads` already matches its policy
```

It did not match. Every file had been created seconds earlier and was waiting out
the cooldown. *Tidy* and *not tidy yet* are the same empty list of operations and
very different sentences, and saying the first when the second is true is how
somebody concludes the thing does not work. `Plan::waiting()` and
`longest_wait()` went into core because both surfaces ask.

---

## 5. Where `--yes` went

`tungstate apply` refuses a big reorganisation without `--yes`. The window does
not have that refusal, and the difference is deliberate:

> A window that has just drawn you two trees has already had the conversation
> `--yes` exists to force.

`--yes` exists because a command can be typed without reading the plan, or run
from a script. So the window shows the same number, `Blast::LIMIT_FILES`, as a
warning *before* the button rather than as a refusal after it — and then asks
once, in a dialog that says what will happen in a sentence.

The one refusal the window keeps is a policy that does not settle, because that
is a bug in the rules rather than a judgement about scale. Nobody should be
offered a button that starts an endless loop.

---

## 6. `include_str!`, `computed`, and one Vue habit worth copying

Three small things for the newcomer's benefit.

**`include_str!` is a macro that runs at compile time.** Rust macros ending in
`!` are expanded before type-checking; this one reads a file relative to the
*source file* it appears in and produces a string literal. If the path is wrong,
the compile fails — which is why the layouts cannot go missing at runtime.

**`computed` in Vue is a cached derivation.** `summary` recomputes only when
`preview` changes, and nothing calls it — the template reads it like a value.
That is the same idea as the planner returning data rather than printing: derive
once, read anywhere.

**Overlays go after the tab chain.** Slice 5b shipped a bug where a `v-if` was
inserted into the middle of a `v-if`/`v-else-if` chain, silently re-chaining two
tabs onto the modal. The comment in this component says where overlays belong
and why, so the next person putting one in does not have to rediscover it.

---

## 7. What is not verified, and why that matters here

The screens in this slice have **not been checked by eye**, and this is the one
slice where that is most nearly unacceptable — three of the last four screen
bugs in this project shipped green, and the whole point of this slice is whether
a person understands what they are looking at.

The window built, launched and ran. What defeated the check was mundane: a
second tungstate was already running from the same binary name, so every window
lookup — by name and by process id — resolved to that one instead, and it is
draining a private library. Rather than keep taking pictures of somebody's
files, the check was abandoned, and in the attempt that window was resized.

The pure functions underneath are tested, which is this crate's stated
convention and is what catches a wrong count or a missing reason. It is not what
catches a white box on a dark sheet, a stretched grid row, or a chain of
`v-else-if` quietly re-parented — and those are precisely the three bugs slice
5b shipped.

**So this slice is not done.** It is pushed, it is green on three platforms, and
it needs somebody to open the window and look at it.
