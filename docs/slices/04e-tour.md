# Tour: slice 4e

The brief is in [04e-connections-in-the-window.md](04e-connections-in-the-window.md).

Five subjects, and only one of them is really about Rust. That is honest for a
slice like this: most of the work was deciding what a string means and making
three places agree about it.

---

## 1. A composite string as a wire format

A pane has one address bar. The "Go to…" list has one `value` per option. The
saved pane state is one JSON string per side. A transfer leg is
`{source, destination, names}`, all strings. A link spec on the command line is
one argument.

So when the window learned about connections, the question was not "how do we
model an endpoint" — the journal already models one, as
`Endpoint { connection: Option<ConnectionId>, path: PathBuf }`. The question was
what crosses the five boundaries above. Two options:

1. Change every one of those shapes to a `{connection, path}` pair.
2. Keep one string, and make the string unambiguous.

We took the second, and the reason is worth stating plainly: option 1 is five
shape changes to express one thing, and each of the five is a place where a
front end and a back end can drift apart with no compiler between them. The
composite string is already the format the user *types*, so making it the
format the program passes around means there is one grammar rather than two.

The grammar is in `connection_prefix`:

```rust
pub fn connection_prefix(raw: &str) -> Option<(&str, &str)> {
    let (name, rest) = raw.split_once(':')?;
    let plausible = name.len() >= 2
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    plausible.then_some((name, rest))
}
```

### Why the two-character rule is load-bearing

`C:\Users\me` is a real path. So is `D:/data`. If "anything before a colon is a
connection name" were the rule, then tungstate on Windows would read a drive
letter as a connection, fail to find it, and tell the user there is no
connection named `C` — for every absolute path they typed.

The rule that makes it safe is arithmetic, not a heuristic: **no drive letter
is ever two characters.** A Windows drive prefix is exactly one letter. So
requiring two or more characters before the colon means the two cases *cannot
collide* — not "rarely collide", cannot. That is the difference between a rule
you can rely on and one that works until someone maps drive `AB:`.

The cost is that a connection cannot be called `x`. That is a real
restriction, and it is the right trade: a one-character connection name would
be unwritable in a link spec anyway, so the restriction removes nothing the
user could have used.

The test that guards it reads the rule back:

```rust
for candidate in [r"C:\Users\x", "D:/data", r"c:\tmp"] {
    let end = parse_end(candidate, None, &journal).unwrap();
    assert!(!end.is_remote(), "`{candidate}` should be local");
}
```

Note `r"C:\Users\x"` — a **raw string literal**. In an ordinary Rust string
`\U` would be an escape (and an error); `r"..."` turns escaping off entirely,
which is what you want any time a literal contains backslashes.

### One more decision inside the grammar

A name that *looks* like a connection and is not one is an error, not a path:

```rust
Some((name, path)) => Ok(Endpoint::remote(lookup(journal, name)?, PathBuf::from(path))),
```

`lookup` returns `EndError::UnknownConnection`, and the message explains the
rule it just applied. The alternative — falling back to treating `nsa:inbox` as
a local directory literally named `nsa:inbox` — would silently create a link
into a folder nobody meant. Typos are more likely than files with colons in
their names, so the error is the useful reading.

## 2. One parser, two front ends, and why the *third* caller is what justified moving it

`describe` and `place` were byte-identical in `cli/src/ends.rs` and
`gui/src/main.rs`. Twenty-eight duplicated lines, and they had been duplicated
since slice 4d.

That duplication was *fine*. Two copies of a small pure function is a cost you
can carry, and the alternative — a shared crate, a new dependency edge, a
public API to keep stable — is a cost too. The rule of thumb that actually
holds up is: duplicate once, extract on the third.

This slice was the third. The window needed `describe` for the Connections tab,
`place` for nothing new, and `parse_end` for `browse`, `places`, `plan_transfer`
and `create_link`. Copying a fourth and fifth call site of a parser into a
second crate is how two front ends start disagreeing about what `nas:` means.

**Where it went, and why that was free.** `tungstate-journal` owns `Endpoint`
and `Connection`. `parse_end` takes a `&Journal` to resolve a name. So the
journal crate already had every type the module needs and already had
`thiserror` for `EndError` — the move added *nothing* to its dependency set.
That is the test worth applying to any "where should this live" question: the
right home is the one where the code costs no new edges.

The CLI's call sites did not change at all:

```rust
use tungstate_journal::{..., ends};
// still: ends::parse_end(&args.from, args.from_connection.as_deref(), journal)
```

Importing the *module* rather than its functions is what made a 200-line move a
two-line diff. It also avoided a real collision: the GUI has its own
`fn describe(error: impl Error) -> String`, and `use tungstate_journal::describe`
would have shadowed it.

## 3. A secret crossing an IPC boundary

`set_connection_password(name: String, secret: String)` takes a password as a
plain `String` over Tauri's IPC. It is worth being precise about what that does
and does not protect, because "the password crosses a boundary" sounds alarming
and the honest answer is more specific than either "it's fine" or "it's not".

**What the boundary is.** Tauri's IPC is in-process message passing between the
webview and the Rust side of the same application. It is not a network socket.
Nothing is listening on a port. The CSP in `tauri.conf.json` is
`default-src 'self'`, so the webview cannot load or contact anything that did
not ship inside the binary.

**What that protects against.** Another program on the machine cannot read the
message. A page on the internet cannot be loaded into this webview and exfil
it. The password is not a command-line argument, so it never lands in shell
history or in `ps` — which is exactly why the CLI has no `--password` flag and
prompts through `rpassword` instead.

**What it does not protect against.** Anything with code execution inside this
process. If an attacker can run JavaScript in the webview, they can read the
form field directly; the IPC hop adds no new exposure, because the secret was
already in the webview's memory the moment the user typed it.

**So the mitigations that matter are the small ones**, and they are all in the
diff:

- It never enters a `tracing` field. `start_transfer` logs its errors; nothing
  on the connection path logs its arguments.
- The Vue side clears the ref after submitting (`secret.value = ""`), so it is
  not sitting in a component that outlives the dialog.
- The *storage* is the machine's keychain, not the journal. `connections.rs`
  says so structurally: there is no column to put a password in.

There is no alternative shape available for a form, and pretending otherwise
would be theatre. What is worth doing is saying which threat each choice
actually addresses — which is this paragraph's whole job.

The one place a real decision was made is the absence of a query:

```rust
/// Offered unconditionally rather than only when one is missing: reading a
/// keychain item from an unsigned binary prompts on macOS, so asking "does
/// this have a password?" would put a system dialog on screen just to decide
/// how to draw a button.
```

A "does this connection have a password?" API would be convenient and would
cost a macOS keychain prompt every time the Connections tab rendered. The UI
offers *Change password* unconditionally instead. Sometimes the right API is
the one you do not add.

## 4. A `Tab` union with a fallback `v-else`, and the bug that shape invites

The tab bar is a TypeScript union and a chain of Vue branches:

```ts
type Tab = "browse" | "transfers" | "connections" | "activity";
```

```html
<template v-else-if="tab === 'browse'">…</template>
<TransfersView v-else-if="tab === 'transfers'" … />
<ConnectionsView v-else-if="tab === 'connections'" … />
<ActivityView v-else />
```

Look at the last line. `ActivityView` has **no condition**. It is the fallback.

This is a shape that quietly punishes you for extending it. Adding
`"connections"` to the union is a type-level change that TypeScript checks:
`tab.value = "connections"` now compiles. But the *template* has no
exhaustiveness checking at all. Had I added the union member and the nav button
and forgotten the branch, the result would be a Connections tab that renders
the Activity view — no error, no warning, no type failure, `npm run build`
green. The bug reports as "the Connections tab shows my file history", which is
a confusing thing to debug precisely because nothing is broken.

Vue has no `match` and no `never` check here, so the mitigation is the comment
that is now in the file:

```html
<!-- Last, and deliberately a bare `v-else`: it is the fallback, so a new
     tab value without its own branch above renders Activity in silence. -->
```

The alternative designs — `<component :is="views[tab]">` with a
`Record<Tab, Component>` map — *would* be checked, because a `Record` over a
union must be exhaustive. That is genuinely better and it is not worth the
churn for four tabs and one new one. The comment buys most of the safety for
none of the diff. Knowing which of those two answers a situation deserves is
most of what "engineering judgment" means in a codebase this size.

The same shape appears in Rust in this project and is checked there, which is
the contrast worth noticing. `Scheme` is a field-less enum and every `match` on
it lists all three variants with no `_` arm — so adding `Scheme::Sftp` in a
later slice will be a compile error in `can_rename`, `is_encrypted`,
`default_port` and four other places, each of which is a real decision someone
has to make. That is the same class of problem as the tab list, and Rust simply
refuses to let it go silent.

## 5. Why `Path::parent` is the wrong question for a remote

This is the smallest function in the slice and the one I would most expect a
reviewer to skim:

```rust
pub fn parent_display(location: &str) -> Option<String> {
    match connection_prefix(location) {
        None => Path::new(location).parent().map(|p| p.display().to_string()),
        Some((name, path)) => match path.trim_end_matches('/') {
            "" => None,
            path => Some(format!("{name}:{}", path.rsplit_once('/').map_or("", |(above, _)| above))),
        },
    }
}
```

`Path::parent` is wrong here **twice over**, and the two reasons are different.

**First, it splits on the wrong separator.** `Path` on Windows treats `\` as a
component separator and `/` as one too; on Unix only `/`. A remote's separator
is `/` on *every* platform, because it is the far side's convention, not this
machine's. So on Windows, `Path::new("nas:inbox/2026").parent()` happens to
work — and `Path::new("nas:inbox").join("a.mp4")` gives `nas:inbox\a.mp4`,
which is not the same key with a different separator. It is a different,
usually corrupt, key, and on some servers it is a directory traversal. This is
the same rule `remote_key` in the OpenDAL adapter and `Location::display_path`
in the journal already encode, for the same reason.

**Second, and more interesting: it answers a question that has no answer.**
`Path::new("nas:inbox").parent()` gives `Some("nas:")`. That looks right. It is
not — `nas:` is not one level above `nas:inbox` in any sense the user would
recognise, it is the connection's root, and `Path` only produced it by
mistaking the `nas:` prefix for a directory component.

Now go one more level: what is the parent of `nas:`? `Path` says `Some("")`.
And `""`, run back through `parse_end`, is a *local* relative path. So a pane
sitting at a connection's root, with an "Up" button wired to `Path::parent`,
would take one click to land somewhere entirely off the connection.

The honest answer is `None`: the root is where the connection's world begins,
and there is nothing above it. `None` is what disables the Up button in
`Pane.vue`, which is exactly the behaviour you want.

The test says it in three lines:

```rust
assert_eq!(parent_display("nas:"), None);
assert_eq!(parent_display("nas:inbox"), Some("nas:".to_string()));
assert_eq!(parent_display("nas:inbox/2026"), Some("nas:inbox".to_string()));
```

### The Rust worth pointing at

```rust
path.rsplit_once('/').map_or("", |(above, _)| above)
```

`rsplit_once` splits at the *last* `/` and returns `Option<(&str, &str)>` — the
part before and the part after. `map_or(default, f)` is `map(f).unwrap_or(default)`
in one call: if there was no `/` at all, the parent within the connection is
the root, which is the empty string. Two lines of branching collapsed into one
expression, and it reads as the sentence "everything above the last slash, or
nothing".

Note also the shadowing in `path => Some(...)`: the match arm binds a *new*
`path` (the trimmed one) over the outer `path`. Rust allows this deliberately,
and it is idiomatic exactly here — the old binding is finished with, and giving
the trimmed value a second name like `trimmed_path` would invite someone to use
the wrong one later.

---

## What this slice did not teach

Almost nothing here was new Rust. `From` impls, a struct with `..spread`
syntax, `map_or`, module re-exports — all of it has appeared before.

That is worth naming rather than apologising for. Most of the work in this
slice was *deciding what things mean*: that a location is one string, that a
name is a key and not a field, that "ready" means listable and not merely
authenticated. Those decisions have no syntax. They show up as a type that
cannot express a rename, a guard written as `source.connection ==
destination.connection` a slice before it could matter, and a readiness poll
that does the thing the caller will do instead of a cheaper thing nearby.

The Rust was the easy part. It usually is, after the first few slices.
