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

## 7. What looking at it found

The screens have now been checked by eye — all twelve states, on macOS, against
a throwaway journal and a throwaway `HOME`. It was worth doing: **eleven
defects**, and only four of them were the ones reading had already found.

### The capture, since last time it was the thing that failed

One window, by its Core Graphics id, filtered to this process's own pid. That is
what makes it safe: nothing else on the screen can land in the file, even when
something is sitting on top of the window. The earlier attempt failed because a
second tungstate held the same binary name; scoping by pid and window id retires
that whole class of problem.

Two things about the method are worth writing down, because both cost time:

- `screencapture -l` captures a window's backing store, and a region capture
  (`-R`) does not — it photographs whatever is *in front of* the rectangle. The
  one region capture taken here caught an unrelated video window and was deleted.
  Window-id capture is not merely tidier, it is the only one that is private.
- A floating window belonging to another app sat at CG layer 3, above everything
  normal, and swallowed clicks aimed underneath it. Moving our window clear of it
  was the fix.

### What reading had found (four)

`.mono` was used three times and defined nowhere — two of the three rendered in
the body font, and the `<pre>` only looked deliberate because browsers default it
to monospace. `.path` was scoped to `.ledger .path`, so the two inline paths in
prose lost it. The selected folder row and the active view toggle both carried
`aria-current` with nothing styling it: on a screen whose job is being easy to
read, neither "the folder you opened" nor "the view you are in" was drawn.

The fix was the generalisation rather than another entry in the list, which is
slice 5b's lesson: `.path` and `.mono` became unscoped rules meaning *this is a
path* and *this is literal text*, and `.ledger .path` kept only the `word-break`
that is genuinely about sitting in a narrow cell. The locbar's path input must
**not** inherit that `word-break` — it ellipsizes on one line — which is exactly
why the split is where it is.

### What widening the audit found (two more)

The same audit, asked about scope rather than existence, found two more of the
identical kind. `.spacer` was defined only under `.topbar` and `.modal .feet`, so
the spacer in the preview's own toolbar matched nothing and the three view
toggles never moved to the right-hand side at all. And a `.ledger` cell carried a
dead `root` token, `.conns .addr.root` being the only rule that mentions it.

Both are the shape this project keeps hitting: **a selector that silently matches
nothing renders as "slightly wrong" rather than as an error.** Nothing fails, so
nothing tells you.

### What only looking could find (five)

1. **Every tab was dead on a fresh install.** `<Welcome v-if="onboarding">` sat
   above the whole `v-else-if` chain, so onboarding replaced the body for *every*
   tab — while the tab bar still moved its `aria-current`. The window said "you
   are on Folders" and showed onboarding. Welcome is the empty state of Browse,
   not of the window, and it is now scoped to that tab. Without this the Folders
   screen is unreachable on a new machine, which is the one thing this slice
   exists to deliver.

2. **A confirmation that rendered as floating prose.** Base `.notice` set padding
   and no background or border, and three views (Folders, Links, Storage) raise a
   bare `note`. Every confirmation in all three read as an indented stray
   sentence. `TransfersView` always passes a modifier, which is the tell that one
   was always intended. The base now carries the container and the modifiers only
   re-tint it.

3. **The window reported operations as files.** Tidying `messy` promised "5 of 6
   files would move" and then said **"moved 9 file(s)"**. `Applied::done` counts
   *operations*, and this plan also made three directories and removed one:
   5 + 3 + 1 = 9. A person who reads both numbers has been told their folder was
   rearranged nearly twice as much as it was.

4. **Undo had the same bug**, from the other end: "put 9 file(s) back". Its count
   now comes from the journal's own ops for that plan, filtered to renames.

5. **A sentence that printed raw seconds.** "The longest has 3351s to wait", on
   the screen whose entire premise is plain words. It reads "51 minutes" now.
   `bytes()` already existed for exactly this reason.

(3) is the one worth dwelling on. It is not a styling slip; it is the window
telling somebody a false number about their own files, and it survived 435 green
tests because nothing tested the *sentence*. The count is now a pure function,
`govern::files_moved`, with the trap written down beside it and two tests: one
asserting the reported number is `blast.files` and explicitly **not**
`plan.ops.len()`, one that a skipped or failed file is not counted as moved.

### What is still not verified

- **Only macOS was looked at.** CI is green on Linux and Windows, and the layout
  is plain CSS grid and flex, but no one has seen those two render. Green is not
  the same as looked at, which is the whole argument of this section.
- **Undo's count is verified by hand, not by a test.** It was seen to say "put 5
  file(s) back" against a plan that reports 9 operations; pinning it needs a
  journal with an applied plan, which is heavier than the helper (3) got.
- The layout picker was seen at both widths; every other state was seen at one.

