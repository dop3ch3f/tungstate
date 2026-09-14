# Tour: slice 5

The brief is in [05-policy.md](05-policy.md).

Two new crates, and the first parser in the project. Five ideas worth slowing
down for, plus one gotcha that cost me a compile error and is worth knowing
before it costs you one.

---

## 1. `serde` and `Spanned`: a library that carries positions but cannot draw them

`serde` is Rust's serialisation framework. You annotate a struct, and a format
crate — `toml` here, `serde_json` elsewhere — fills it in:

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Folder {
    pub name: Spanned<String>,
    pub mode: Option<Spanned<Mode>>,
    ...
}
```

Three things are doing work in those four lines.

**`#[serde(deny_unknown_fields)]`** turns a typo into an error instead of
silence. Without it, `priorty = 3` is ignored and the reader spends an hour
wondering why their priority is not respected. With it:

```
× unknown field `priorty`, expected one of `name`, `path`, `match`, `vars`, `rename`
```

This is also how `priority` — a field this slice deliberately removed — reports
itself as gone rather than being quietly dropped.

**`Option<T>` means "may be absent"**, and it is the difference between "the
user did not say" and "the user said the default". We need that difference for
`mode`, because a policy that omits it should get `observe` and a policy that
writes `mode = "observe"` should get the same answer by a different route. If
the field were a bare `Mode` with `#[serde(default)]`, the two would be
indistinguishable — fine here, but the habit is what keeps `Option<Option<T>>`
patch semantics reachable later.

**`Spanned<String>` is the interesting one.** It is a newtype from
`serde_spanned` that wraps a value and records *where in the input it came
from*:

```rust
pub struct Spanned<T> { /* private */ }
impl<T> Spanned<T> {
    pub fn span(&self) -> Range<usize>;   // byte offsets into the source text
    pub fn get_ref(&self) -> &T;
    pub fn into_inner(self) -> T;
}
```

A **newtype** is Rust's cheapest abstraction: a struct wrapping exactly one
value, costing nothing at runtime, existing purely so the type system knows
something extra. `FolderId(pub String)` in `tungstate-api` is the same trick for
a different purpose.

So why go to the trouble? Because of this line in DESIGN §7:

> `thiserror` in libraries, `miette` in the CLI.

`tungstate-core` reports a problem as data:

```rust
PolicyError::Template {
    rule: String,
    message: String,
    span: Range<usize>,   // just two numbers
}
```

and knows nothing about terminals, colour, or how to underline a line of text.
The CLI turns those two numbers into this:

```
  × rule `archives`: `shout` is not a filter; the filters are lower, upper,
  │ slug, sanitize, trunc:N, pad:N, hash:N and default:"x"
    ╭─[bad.toml:25:23]
 24 │ name = "archives"
 25 │ path = "Archives/{ext|shout}"
    ·                       ──┬──
    ·                         ╰── here
 26 │ match = { ext = ["zip", "tar", "gz", "7z"] }
    ╰────
```

The payoff is not aesthetic. `miette` pulls in a terminal renderer, colour
detection and unicode width tables — none of which a policy *model* should
depend on, and all of which are wrong for the desktop app, which will want to
put a red squiggle in a text editor rather than draw box characters. **A span
kept as data can be rendered three ways. A span already rendered can be
rendered one.**

There is a real cost, and it is worth naming: the two-layer shape. The `raw`
module mirrors the TOML exactly, `Spanned` everywhere; then `compile()` walks it
into the public types, which hold compiled globs and regexes and only the spans a
trace still needs. That is two structs per table instead of one. The reason it
earns its keep is that `Regex::new` is fallible and expensive, and you want it to
happen **once at load**, with a span to point at when it fails — not on every
file.

## 2. Writing a parser by hand, and when that is the right call

`{date:%Y}/{source|default:"unknown"}/{category}` has a grammar. Four
productions:

```
template    := (literal | placeholder)*
placeholder := '{' name (':' format)? ('|' filter)* '}'
filter      := name (':' arg)?
literal     := anything else, with '{{' and '}}' as escaped braces
```

The instinct from other ecosystems is to reach for a parser generator — `pest`,
`nom`, `lalrpop`. All three are good. None is right here, for three reasons.

**The grammar is finished.** It has four productions and it is not going to
grow, because the escape hatch for anything more complicated is already
designed and is Rhai (§2, out of scope for v1). A parser generator pays off when
a grammar will keep changing; this one will not.

**A generator is a build step between the reader and the rules.** With `pest`,
understanding what `{a|b:3}` does means reading a `.pest` file, then the
generated visitor, then the code. Here it means reading one 60-line function.

**The error messages are the product.** `explain` exists to answer "why did it
go there", and half the answers are "because your template says something other
than what you think". A generator's errors are about its grammar
(`expected rule filter_name`); hand-written errors are about the user's intent
(`` `shout` is not a filter; the filters are ... ``). Slice 5's entire reason
for existing makes that trade one-sided.

The parser itself is a character loop:

```rust
let mut chars = source.char_indices().peekable();
while let Some((at, c)) = chars.next() {
    match c {
        '{' if chars.peek().is_some_and(|(_, n)| *n == '{') => {
            chars.next();
            literal.push('{');
        }
        ...
    }
}
```

`char_indices()` yields `(byte_offset, char)` pairs — **byte** offset, not
character index, which is what `Spanned` and `miette` both want, and what makes
slicing `&source[a..b]` valid. `peekable()` adds one item of lookahead, which is
all this grammar needs: `{{` is an escaped brace and `{` is a placeholder, and
one character tells them apart.

A **match guard** (`if chars.peek()...` after the pattern) is the piece of
syntax worth noticing. It lets a match arm depend on something beyond the
pattern's shape. The alternative is a nested `if` inside the arm, which then has
to fall through to the other cases by hand — and in Rust it cannot, because
match arms do not fall through. The guard is how you say "this arm, but only
when".

### The bug this shape invited, and the fix

The first version found the placeholder's end with "the next `}`":

```rust
let Some(close) = source[at..].find('}').map(|i| at + i) else { ... };
let body = &source[at + 1..close];
```

For `rename = "{date:%Y.{ext}"` — an unterminated placeholder with another one
after it — that finds the `}` belonging to `{ext}`, takes `date:%Y.{ext` as a
variable name, and reports:

```
unknown variable `date`
```

which is true, useless, and points at the wrong problem. One check fixes it:

```rust
// A `{` inside the body means this placeholder was never closed and the
// `}` found belongs to the next one.
if body.contains('{') { return Err(/* "`{` is never closed" */); }
```

The test that caught it is a snapshot, and the diff in review was the whole
value of having one: the old expectation was visibly a worse message than the
new one.

## 3. An enum as a cost model

```rust
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier { Stat, Head, Meta, Whole }   // free · 8 KiB · 64 KiB · everything
```

`PartialOrd, Ord` on a field-less enum orders the variants **by declaration
order**. `Stat < Head < Meta < Whole` is true because that is how they are
written, and there is a test asserting exactly that, because someone reordering
them for tidiness would silently invert the cost model.

That ordering is the whole machine. A policy's tier is a fold:

```rust
pub fn required_tier(&self) -> Tier {
    self.rules.iter().map(|rule| rule.tier).max().unwrap_or(Tier::Stat)
}
```

`max()` over an `Iterator` needs `Ord`; `unwrap_or` handles a policy with no
rules, where the honest answer is "read nothing". And each rule's tier is the
same fold over its matcher's attributes, its vars' sources, and its templates'
placeholders — every name run through one function:

```rust
pub fn tier_of(attribute: &str) -> Option<Tier> {
    match attribute {
        "name" | "stem" | "ext" | ... => Some(Tier::Stat),
        "mime" => Some(Tier::Head),
        "hash" => Some(Tier::Whole),
        other if other.strip_prefix("exif.").is_some_and(|t| !t.is_empty()) => Some(Tier::Meta),
        _ => None,
    }
}
```

The `None` case is doing double duty: it is "this costs nothing" *and* "this is
not an attribute I know", and the loader uses it for the second meaning to
reject `vars.d = { from = "colour" }` with a span. One function, two callers,
no way for them to disagree about the vocabulary.

`{name|hash:8}` is the case that shows the model is about *attributes* rather
than about spelling. The `hash:8` there is a filter that hashes the variable's
text, not the `hash` attribute that reads the file, so a rename using it stays
at tier `stat`. There is a test for that, because the two look identical at a
glance.

### What it buys, measured

The `stat` row in the brief's table is the one to internalise: **a policy that
asks only about names, sizes and times never opens a file.** Against a 658 MB
video over FTP, the server logged no data transfer at all. Governing a NAS full
of video by extension and size costs listings and nothing else.

And the honest limit, which the brief records in full: over FTP a `head` tier
asked for 8 KiB and 1.1 MB arrived, because FTP has no way to request a range
*with an end*. `REST` sets a start; ending a `RETR` early means closing the data
connection after the server has already pushed its socket buffers. A 400×
saving, not an 800× one. Over HTTP, S3 and WebDAV a `Range` header is exact.

Which is why the FTP test asserts the thing that is true — the file did not come
across — rather than a byte count that belongs to the kernel. **A test that
asserts more than the system promises is a test that will fail for a reason
nobody can act on.**

## 4. A defaulted trait method, again, and why it keeps being the answer

Slice 4f used this and so does this one:

```rust
pub trait Backend: Send + Sync {
    /// Read at most `len` bytes from the start of `path`.
    fn read_prefix(&self, path: &Path, len: u64) -> Result<Vec<u8>> {
        let mut reader = self.open_read(path)?.take(len);
        ...
    }
}
```

A trait method with a body is a method implementors *may* override and need
not. The three consequences here:

- **Six existing implementations kept compiling.** There are five test doubles
  in `tungstate-transfer` and the `Doorman` in `governor.rs`; a required method
  would have meant editing all six to add something none of them cares about.
- **The default is correct, not a stub.** It opens the file and truncates, which
  answers the question — just without the saving. A backend that cannot do
  ranges is *slow*, never *wrong*. That distinction is what makes defaulting
  safe here and unsafe in general: a default that returns `unimplemented!()` is
  a runtime bomb wearing a compile-time disguise.
- **The two that can do better, do.** `LocalBackend` uses `File::take`, and
  `OpendalBackend` issues a real ranged request.

`.take(n)` is worth a note in passing. It is an adaptor on `Read` that wraps a
reader and stops it after `n` bytes — the same idea as `Iterator::take`, and a
good example of Rust's composition style: rather than every reader growing a
`read_at_most` method, one wrapper adds the behaviour to all of them.

The `OpendalBackend` override needed one thing the local one did not:

```rust
match operator.read_with(&owned).range(0..len).await {
    // A range past the end means the file is shorter than the prefix, so
    // reading it whole costs fewer bytes than asked.
    Err(error) if error.kind() == ErrorKind::RangeNotSatisfied => operator.read(&owned).await,
    other => other,
}
```

A 200-byte file and an 8 KiB prefix is not an error, it is a short file — but
`OpenDAL` reports `RangeNotSatisfied`, and the test that reads six prefix
lengths including one past the end is what found it. The three implementations
are asserted to agree byte for byte at every length, which is the only way to
know a defaulted method and its overrides have not drifted.

## 5. Property tests, and the one invariant worth having

A **property test** asserts something true of *all* inputs, and the framework
(`proptest`) generates inputs trying to break it, then shrinks any failure to
the smallest case that still fails.

The first property is cheap and mostly guards against accidents:

```rust
#[test]
fn classification_is_deterministic(path in template(), rename in template(), attrs in attributes()) {
    let policy = generated_policy(&path, &rename);
    prop_assert_eq!(policy.explain(&attrs), policy.explain(&attrs));
}
```

If that ever fails, something reached for a clock, a hash map's iteration order,
or a global. It is the test that keeps "pure function of attributes and policy"
from quietly becoming untrue — which is why `Attributes` carries `now` as a
field instead of calling `Timestamp::now()` inside the classifier, and why every
map in the model is a `BTreeMap` rather than a `HashMap`. `BTreeMap` iterates in
key order; `HashMap` iterates in an order that is deliberately randomised per
process. For anything that reaches output, ordered is the only defensible
choice.

The second property is the one that matters:

```rust
#[test]
fn a_destination_is_always_relative_and_never_escapes(...) {
    if let Outcome::Routed { destination, .. } = &trace.outcome {
        prop_assert!(!destination.starts_with('/'));
        for component in Path::new(destination).components() {
            prop_assert!(matches!(component, Component::Normal(_)));
        }
        ...
    }
}
```

**A filename is attacker-controlled input, and it flows into a path.** Someone
sends you `../../.ssh/authorized_keys.jpg`; a template says
`{date:%Y}/{name}`; and without this invariant a governed folder is an
arbitrary-file-write primitive. So the generator is built to attack it: the
templates it produces contain `..`, `/../` and backslashes as literal runs, and
the attributes it produces include an EXIF value of `../Evil\Corp`.

What makes it hold is `canonical_path`, which is deliberately not an escaping
function:

```rust
rendered
    .split(['/', '\\'])
    .map(str::trim)
    .filter(|s| !s.is_empty() && *s != "." && *s != "..")
    .map(sanitize)
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("/")
```

`..` is **dropped**, not escaped or rejected. Dropping is the choice that cannot
be got wrong later: an escaping scheme has to be undone somewhere, and the
place it gets undone is the vulnerability. `LocalBackend::resolve` already
refuses `..` as a second line of defence — this is the first, and defence in
depth here is cheap.

`sanitize` runs on every segment whether or not the policy asks for it, for the
same reason: a filename illegal on the target is not a preference. It takes the
*union* of the platforms' rules — no `<>:"/\|?*`, no control characters, no
trailing dot or space, no `CON`-style device names — because a NAS reached over
SMB serves Windows clients whatever it runs itself. Writing `|sanitize`
anywhere but last in a chain is a load-time error, since it would read as if it
mattered.

That rule is also why bucket labels are words. `<10MiB` sanitises to `_10MiB`
and so does `>10MiB`, which would put two disjoint size buckets in the same
folder — a silent data-merging bug born of a cosmetic decision. The policy is
still written `["<10MiB", "10MiB-1GiB", ">1GiB"]`, and `Interval::label` turns
those into `under-10MiB`, `10MiB-1GiB`, `over-1GiB`. There is a test asserting
each label survives `sanitize` unchanged, which is the property that actually
matters.

## 6. The gotcha: `thiserror` owns the name `source`

This will not compile:

```rust
#[derive(Debug, thiserror::Error, Diagnostic)]
struct PolicyDiagnostic {
    #[source_code]
    source: NamedSource<String>,   // the *text*, not a cause
}
```

```
error[E0599]: the method `as_dyn_error` exists for struct `NamedSource<String>`,
              but its trait bounds were not satisfied
```

`thiserror` treats a field literally named `source` as the error's underlying
cause — the thing `Error::source()` returns — without needing an attribute, by
convention. `NamedSource` is not an error, so the generated code asks it for
something it cannot provide, and the message points at the trait bound rather
than at the naming rule.

Renaming the field to `text` fixes it. The lesson generalises past this one
crate: **derive macros claim names, and the error you get is about the code they
generated, not about the rule you broke.** When a derive produces a message
mentioning a method you never wrote, suspect a magic field name before
suspecting the types.

## 7. What is deliberately not here

`explain` computes where a file *would* go. Nothing scans, nothing plans,
nothing moves — that is slices 6 and 7, and the `Explanation` this slice
produces is the input the planner will consume, which is why the trace is the
decision rather than a debugging view bolted onto it.

`strict` and `ensure` on directories are absent for a sharper reason than
"later": they describe **drift**, and drift is a statement about what is in a
directory versus what should be. There is no index yet, so there is nothing to
compare against, and a `strict = true` that could not detect anything would be a
field that lies.
