# Slice 5: the policy model, the classifier, and `explain`

**Goal:** tungstate stops being a transfer tool and becomes the thing the
design pitches — a reconciliation controller for filesystems. You declare the
shape a tree should have; tungstate computes where each file belongs as a pure
function of its attributes and the policy.

Nothing is moved here. What ships is the declaration, the decision, and the
ability to interrogate the decision.

**Runnable outcome:** write a `.tungstate/policy.toml`, point
`tungstate explain <FILE>` at a photo, and read the trace — every rule in file
order, whether it matched and which constraint decided it, then each variable,
where its value came from and what reading it cost, and the rendered path with
each filter's effect.

```
$ tungstate explain ~/Downloads/IMG_0001.JPG
IMG_0001.JPG
  policy .tungstate/policy.toml (folder "downloads"), reads every file in full (tier: whole)

rules, in file order
   1. media-by-origin  (line 12)   matched
   2. archives         (line 21)   no: ext — `jpg` is not one of `zip`, `tar`, `gz`, `7z`

variables for `media-by-origin`
  date        = 2023-12-25T08:30:00      from exif.DateTimeOriginal (meta), tz utc
  category    = Photos                   from mime (head), = image/jpeg, map `image/*` = "Photos"
  size_range  = under-10MiB              from size (stat), = 4194304, bucket `under-10MiB`
  ...

destination  2023/12/Photos/under-10MiB/20231225_083000_ce57a60f.jpg
```

`explain` is the deliverable that matters. DESIGN §2: *"Build this from day
one; it's also your debugger."*

**This brief is the spec.** The tour is in [05-tour.md](05-tour.md).

---

## Decisions

### First match in file order wins

**Chosen.** Rules are tried top to bottom and the first whose `match` succeeds
takes the file.

**Rejected: most-specific-match-wins with a `priority` tiebreak and a load-time
ambiguity error**, which is what DESIGN §2 originally specified. Two reasons it
lost. A specificity metric over globs, regexes, mime wildcards and size ranges
is a heuristic dressed as a rule, and the first time it disagrees with the
reader it costs more than it saves. And file order is the one ordering every
reader already understands without documentation.

The argument that makes the simpler rule *safe* is the user's, and is recorded
here and in DESIGN §2 because it is the load-bearing part:

> Reconciliation is convergent, so a mis-ordered rule is not a permanent
> mistake — reverting the rule reverts the structure, the way a revert does in
> git.

That is precisely what Hazel and friends cannot offer, and it is why the usual
objection to first-match-wins does not land here. There is a test for it:
`reordering_two_rules_changes_where_a_file_goes` moves a rule, watches the
destination change, and moves it back.

### A shadowing warning replaces the ambiguity error

Losing the load-time ambiguity error entirely would reintroduce the silent
surprise it existed to prevent. So one thing is added back in its place:

> If rule *n* can never fire because an earlier rule subsumes it, the loader
> says so, naming both lines. **A warning, not an error** — the policy is
> usable, and a rule that can never fire is a thing to know rather than a thing
> to stop on.

Exact for literal sets (`ext`, `mime`) and for size and age intervals. **Silent
whenever a glob or regex is involved on the earlier side**, since deciding
whether two regexes overlap is not a thing a policy loader should attempt, and
a wrong warning is worse than no warning.

### `[[rule]]`, and no `priority` field

§2 is the later DECIDED spelling; §1's `[[dir]]` prose was stale and is fixed.
`priority` is gone, because file order *is* priority. A policy that still has
one gets `unknown field 'priority'` with the line, which is the right failure.

### `tungstate-attrs` is a separate crate

DESIGN §7 puts the policy model, classifier and planner in `tungstate-core` and
says its having **zero I/O is the load-bearing decision for testability**.
Gathering attributes is I/O by definition, so it cannot live there. It gets its
own crate, and core's promise survives: the whole classifier is property-tested
without a filesystem.

### `--json` on `explain` only

The convention gets established where it pays. Every other command stays human
output until something needs otherwise.

### Bucket labels are spelled in words

`<10MiB` is not a legal directory name on Windows or over SMB, and sanitising
`<` and `>` to `_` would make `<1GiB` and `>1GiB` the same folder. So a bucket
label is derived: `under-10MiB`, `10MiB-1GiB`, `over-1GiB`. The policy is still
written with the comparison spelling, which is the readable one.

---

## What ships

### 1. `crates/tungstate-core` — the policy model and classifier, zero I/O

`serde` structs mirroring the DESIGN §2 sketch, every value a diagnostic might
point at wrapped in `toml::Spanned`:

```toml
[folder]
name = "downloads"
mode = "observe" | "suggest" | "enforce"
inbox = "_inbox"
ignore = [".DS_Store", "*.part", ".git/**"]
opaque = ["*.app", "**/node_modules"]
symlinks = "ignore" | "treat-as-file" | "follow-within-folder"

[defaults]
on_duplicate = "trash"
on_conflict = "quarantine"
cooldown = "30s"

[[rule]]
name = "media-by-origin"
path = "{date:%Y}/{date:%m}/{source}/{category}/{ext}/{size_range}"
match = { mime = ["image/*", "video/*"] }
vars.date       = { from = ["exif.DateTimeOriginal", "mtime"], tz = "local" }
vars.category   = { from = "mime", map = { "image/*" = "Photos", "video/*" = "Videos" } }
vars.size_range = { from = "size", bucket = ["<10MiB", "10MiB-1GiB", ">1GiB"] }
rename = "{date:%Y%m%d_%H%M%S}_{hash:8}.{ext|lower}"
```

**Matching.** `globset` for globs, `regex` for captures (named captures become
variables), literal sets for `ext` and `mime` with `image/*` wildcards, a size
grammar (`> 1MiB`, `< 10MiB`, `1MiB..1GiB`) and an age grammar (`> 30d`). A
rule with no `match` takes everything left, which is how a trailing catch-all
is written.

**Variables.** `vars.<name>.from` is one attribute or a fallback chain; the
first that resolves wins, and the trace says which were skipped. `map` relabels
text, taking `"_"` as its catch-all and preferring an exact key, then the
wildcard with the most literal text. `bucket` relabels a size, parsing the same
grammar as `match.size`.

**Templates and filters.** One hand-written parser for `{var}`,
`{var:%Y-%m-%d}` and `{var|filter|filter}`. The DECIDED v1 filter set, no more:
`lower`, `upper`, `slug`, `sanitize`, `trunc:N`, `pad:N`, `hash:N`,
`default:"x"`, and strftime specifiers on dates. **`sanitize` is applied last to
every path segment regardless of whether it is written**, because a filename
illegal on the target backend is not a preference. Writing it anywhere but last
is refused at load time, since it would read as if it mattered.

**Errors.** `PolicyError` (thiserror) carrying byte spans as plain
`Range<usize>`. The crate does not depend on `miette`; §7 puts `thiserror` in
libraries and `miette` in the CLI, and keeping the span as data is what lets the
GUI render it differently later.

### 2. `crates/tungstate-attrs` — gathering attributes, tier by tier

```rust
/// How much of a file has to be read to answer a policy's questions.
pub enum Tier { Stat, Head, Meta, Whole }   // free · 8 KiB · 64 KiB · everything

pub fn gather(backend: &dyn Backend, path: &Path, tier: Tier) -> Result<Attributes>;
```

`Policy::required_tier()` in core is the maximum over every attribute any rule
references, so a policy that never mentions `exif` or `hash` never pays for
them. Over FTP that is the difference between a ranged read and pulling a 4 GB
video across the network to decide it is a video.

Extractors: `mime` from magic bytes (`infer`, extension fallback, then a
printable-bytes heuristic), `exif.*` (`kamadak-exif`), `hash` (`blake3`). Dates
go through `jiff`, which bundles a tzdb so `tz = "Africa/Lagos"` means the same
thing on every platform.

### 3. `Backend` gains a prefix read

The tiers are decorative without it: `open_read` has no offset, so "read the
first 8 KiB" would mean streaming the whole file.

```rust
/// Read at most `len` bytes from the start of `path`.
///
/// Defaulted rather than required: a backend that cannot ask the far side for
/// a range still answers correctly by truncating, and every test double keeps
/// compiling.
fn read_prefix(&self, path: &Path, len: u64) -> Result<Vec<u8>> { /* read + truncate */ }
```

`LocalBackend` overrides with `File::take`. `OpendalBackend` overrides with a
ranged read, which is a real `REST`/`RETR` over FTP, falling back to a whole
read when the range runs past the end of a short file. A defaulted trait method
is the same technique slice 4f used, for the same reason.

### 4. `tungstate-cli` — `explain`, and `policy validate`

```
tungstate explain <TARGET> [--policy <FILE>] [--json]
tungstate policy validate [--policy <FILE>]
```

`TARGET` is a path, or `connection:path` — the same spelling `link add` takes,
through the same `parse_end`. For a local file `explain` finds the policy by
walking up to a `.tungstate/policy.toml`, with `--policy` overriding, so it
works before governed folders exist and `folder add` can stay the stub it is.

For a file on a connection the policy must be named: walking a remote tree
looking for `.tungstate/policy.toml` is a round trip per level, and §2 puts a
remote folder's policy in the central config directory rather than in the
folder anyway. The connection's own root stands in for the governed folder, so
`{parent}` means there what it means locally.

`--json` emits the same trace as a structure, with each rule's line number
added beside its span. `policy validate` exists because once the parser is
written it is twenty lines, and it gives the diagnostics somewhere to live
other than a failing `explain`.

`miette` (with `fancy`) enters here and only here, wrapping `PolicyError`'s
spans in a `NamedSource` so a bad policy renders as §7 asks, pointing at the
line.

### 5. Dependencies

New to `[workspace.dependencies]`: `miette` (CLI only), `globset`, `toml` (1.x,
for `Spanned`), `kamadak-exif`. Promoted from transitive to direct, so no new
compilation: `regex`, `infer`, `jiff`. `Cargo.lock` is committed; CI runs
`--locked`.

---

## Acceptance criteria

- `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`,
  `cargo test --locked` clean, plus `npm run build` in `crates/tungstate-gui/ui`.
- A real photo with EXIF routed by its camera date, not its mtime.
- A policy broken four ways, each diagnostic pointing at the right line.
- Two rules reordered, the explained destination changing, and reverting the
  reorder reverting the answer.
- `explain` over FTP reading a prefix rather than the file, confirmed from the
  server's own log.
- CI green on Linux, macOS and Windows.

### What the FTP measurement actually showed

Worth writing down, because the obvious reading of "tier" is wrong for this one
protocol. Against a 658 MB video on a real vsftpd, reading the server's own log:

| policy tier | asked for | crossed the wire |
|---|---|---|
| `stat` | nothing | **no data transfer at all** |
| `head` | 8 KiB | 1.1 MB |
| `meta` | 64 KiB | 1.6 MB |

The `stat` row is exact and is the one that matters most: a policy that asks
only about names, sizes and times never opens a file, so governing a NAS full
of video costs listings and nothing else.

The other two are a 400× saving and not the number on the tin. **FTP has no way
to ask for a byte range with an end.** `REST` sets a start offset; the only way
to finish a `RETR` early is to close the data connection, by which point the
server has already pushed whatever fit in the socket buffers. So over FTP a
tier is a bound on what is *requested*, not on what arrives. Over HTTP, S3 and
WebDAV a `Range` header is exact, and over SFTP a read at offset plus length
is exact; FTP is the outlier, and it is the one this project cares most about.

The FTP test asserts the honest thing — the file did not come across — rather
than a byte count that belongs to the kernel.

## Tests

**Core, unit** — every filter including chains and `sanitize` running last;
every bucket boundary including both edges; `map` with and without `"_"`;
strftime against `local`, `utc` and `Africa/Lagos`; the size and age grammars
including malformed input; `from` fallback chains where the first source is
absent; first-match-wins ordering; the shadowing warning firing on literal
overlap and staying silent on globs.

**Core, property (`proptest`)** — the two invariants slice 5 can reach:
classification is deterministic for the same attributes and policy; and **a
rendered canonical path is always relative and never escapes the root**,
whatever the template and whatever the filename. The second is the one that
matters, since a filename is attacker-controlled input and it flows into a
path. The generator deliberately produces templates containing `..`, `/../` and
backslashes, and attributes whose EXIF values contain `../Evil\Corp`.

**Parser diagnostics** — a bad `mode`, an unknown filter, an unterminated `{`
and an unknown variable each report the right line, plus ten more loader
errors, rendered through `insta` so a regression in a diagnostic is visible in
review.

**Attributes** — `read_prefix`'s default implementation and both overrides
agree on the same file at six lengths including past the end. And the tests
that prove the tiers are real rather than decorative: a counting backend that
records every read, asserting a mime-only policy never reads past 8 KiB of a
4 MiB file, and that a policy mentioning neither `exif` nor `hash` never opens
the file at all.

**CLI** — `explain` on a fixture tree through the existing `sandboxed()`
harness, with the human trace snapshotted; `--json` parsed back and asserted
field by field; the reorder demonstration; `policy validate` exiting 0 on a good
policy and 1 with a line-pointed diagnostic on each of four broken ones; a
shadowed rule warning without failing; and `explain` through a connection over
the `fs` scheme, which needs no server and so covers the remote path on all
three platforms rather than only on the Linux FTP job.

**FTP** — `explain_over_ftp_reads_a_prefix_rather_than_the_file`: a stat-tier
policy against an 8 MiB remote file, then a head-tier one, asserting the
destination is right, that `explain` moves nothing, and that the head-tier run
finishes in a time a whole-file read could not.

**Known breakage:** `cli__help_output_is_stable.snap` changes when `explain`
and `policy` appear. Expected, and reviewed rather than blind-accepted.

## Out of scope

- **Applying anything.** No index, no scan, no planner, no moves. `explain`
  computes where a file *would* go. Slices 6 and 7.
- `strict` and `ensure` on directories — they describe drift, which needs the
  planner to mean anything.
- `{source}` resolves from the journal and works; `id3.*`, `pdf.title` and
  `video.duration` do not. They plug into the same extractor seam later; the
  tier machinery is built for them.
- `tungstate init --template`, the `policies/` directory of built-in presets,
  and `policy fmt`. Realistic policies live in test fixtures for now.
- `--json` on any command but `explain`.
- The Rhai escape hatch. Not in v1, by design.
