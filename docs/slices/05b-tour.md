# Tour: slice 5b

The brief is in [05b-the-window-catches-up.md](05b-the-window-catches-up.md).

No new crates this time, and the Rust is mostly things you have already met.
What is worth slowing down for here is a different kind of lesson: four
decisions about *data* that each turned out to be the whole feature, and three
bugs that got past a green test suite because nobody opened the window.

---

## 1. A tombstone column, and why not the one that was already there

The problem is small to state. You want to delete a link. The journal's
operations refer to their link by id:

```sql
CREATE TABLE ops ( ..., link_id INTEGER );
```

so deleting the row leaves `tungstate log` and `tungstate whereis` naming an id
that nothing can resolve. The history survives and stops making sense, which is
worse than not being able to delete at all.

The answer is a **tombstone**: a column that marks a row as gone without
removing it. The interesting part is which column.

There was already a `saved` column, added in migration v3, distinguishing a
named pair you made on purpose from the throwaway one a browser transfer
creates. Reusing it would have been one fewer migration. The reason not to is in
the migration's own comment (`crates/tungstate-journal/src/schema.rs:104`):

```sql
-- `deleted_at` rather than reusing `saved`: `saved` already answers a
-- different question (a named pair, or the one-off a browser transfer
-- makes), and a column that answers two questions answers neither.
ALTER TABLE links ADD COLUMN deleted_at INTEGER;
```

**A column that answers two questions answers neither.** Once `saved = 0` means
either "this was a one-off" or "this was retired", every query that reads it has
to know which it meant, and the two meanings drift apart the first time someone
adds a third caller.

Two more properties fall out of the choice, and both are free:

- **It records *when*.** `deleted_at INTEGER` holds milliseconds since the Unix
  epoch, so a retired link can say when it was put away. A boolean cannot.
- **NULL means live.** Every row that already existed is correct the moment the
  migration runs, with no backfill. `ALTER TABLE ... ADD COLUMN` without a
  `NOT NULL` fills existing rows with NULL, and NULL is exactly the answer we
  want for them. That is why the migration is one line and why it cannot half-fail.

In Rust the column is `Option<i64>`:

```rust
pub struct Link {
    pub deleted_at: Option<i64>,
    ...
}
```

`Option<T>` is Rust's way of saying "there may or may not be a value here", and
it is a real enum with two variants — `Some(value)` and `None` — not a pointer
that might be null. The compiler will not let you read the `i64` without first
saying what you want to happen when there is none. That is the whole of Rust's
answer to the null-pointer problem: the absence is in the type, so it cannot be
forgotten.

---

## 2. Delete or retire, decided by the data, not by a flag

`Journal::remove_link` (`crates/tungstate-journal/src/links.rs:438`) does not
take a `--force` or a `retire: bool`. It looks at what is true and decides:

```rust
pub fn remove_link(&self, name: &str) -> Result<Removal> {
```

and `Removal` is a two-variant enum reporting which happened, so both surfaces
can say the right thing:

```rust
pub enum Removal {
    /// The row is gone. Only for a link that never ran, so nothing refers to it.
    Deleted,
    /// The row stays, marked with the time it was put away, so the operations
    /// that name it by id keep resolving. Its name is released.
    Retired,
}
```

Returning an enum rather than `()` is the pattern worth taking away. The caller
has to `match` on it — there is no way to ignore the distinction by accident —
and the two arms in the CLI print two different sentences. A `bool` would have
worked and would have read as `if removed { ... }` at every call site, which is
the kind of line you misread at a glance.

The body does three queries in order, and the order is the design:

**First, refuse if work is unfinished.**

```rust
// Refused rather than handled, for the same reason `connection remove`
// is refused while a link points at a connection: removing this now
// would strand a part-copied file at the destination with nothing left
// able to name it. `link run` or `link discard` first.
let unfinished: i64 = conn.query_row(
    "SELECT COUNT(*) FROM ops WHERE link_id = ?1 AND status = 'intended'", ...
```

An `intended` op is one the journal wrote *before* touching the filesystem and
never closed — so there may be a half-copied temp file at the destination right
now. The link row is the only thing that knows where that destination is. Delete
it and the file is orphaned for ever, invisible to every command. Refusing is
not caution; it is the only answer that does not lose data.

**Second, delete outright if there is no history.**

```rust
if history == 0 {
    conn.execute("DELETE FROM links WHERE id = ?1", ...)?;
    return Ok(Removal::Deleted);
}
```

Nothing refers to it, so nothing breaks. A link you made and never ran is just
gone, which is what you expect from a delete button.

**Third, retire, and release the name.**

```rust
// The name is released along with the retirement. Keeping it reserved
// would mean an invisible row refusing a name the user can see is
// free, which is a worse surprise than a retired link reading as
// `nas#3` in the one place retired links are ever shown.
conn.execute(
    "UPDATE links SET deleted_at = ?2, name = name || '#' || id WHERE id = ?1", ...
```

`name || '#' || id` is SQL string concatenation: `nas` becomes `nas#3`. The name
`nas` is now free to use again, and the retired row is still findable by id.

### The asymmetry that makes it work

Three functions read the links table, and they deliberately do not agree:

```rust
// link_by_name — live only
"SELECT * FROM links WHERE name = ?1 AND deleted_at IS NULL"

// links() — live, saved ones only
"SELECT * FROM links WHERE saved = 1 AND deleted_at IS NULL ORDER BY id"

// link_by_id — no filter at all
```

`link_by_id` finding retired links is not an oversight; it is the point. Asking
by *name* is asking "what should I offer the user", and a retired link is not
that. Asking by *id* is asking "what does this operation refer to", and the
answer must still exist or the history breaks. Same table, two questions, two
queries — which is exactly the discipline section 1 refused to give up by
overloading `saved`.

---

## 3. One rule, two surfaces

This one shipped as a bug fix during the slice, and it is the most instructive
thing in it.

The conflict setting — what to do when a file of that name is already at the
destination — is offered in both the window's transfer dialog and on the command
line. The window labels the options as *decisions*: "keep both, renaming the
arriving file", "leave that one here", and only quarantine as "ask me, and set
aside if I am away".

`WindowResolver::resolve` nevertheless emitted a prompt every time, treating the
stored setting as a *fallback* for when nobody answered. So choosing an action
and then being asked about it contradicted the words on the screen. And because
**every prompt holds a worker until it is answered**, an unattended drain
stopped at the first clash and waited. The command line had the same hole from
the other side: `link run x` on a terminal prompted regardless of the link's
stored rule.

Two surfaces, one question, two different answers. The fix was to make it
impossible for them to disagree, by putting the rule where both already look:

```rust
impl ConflictAction {
    /// Whether a person should be asked, rather than this simply applied.
    ///
    /// Only `Quarantine`, which is the one both surfaces describe as *ask me,
    /// and set aside if I am away*. The others are decisions: someone who
    /// chose "keep both, renaming" has said what they want and asking again
    /// contradicts the words they read.
    #[must_use]
    pub fn wants_asking(self) -> bool {
        matches!(self, Self::Quarantine)
    }
}
```

Three small Rust things in six lines:

- **`self`, not `&self`.** `ConflictAction` derives `Copy`, so it is a plain
  integer-sized value that is duplicated rather than moved. Taking it by value
  is cheaper than taking a reference to it and reads better at the call site.
- **`matches!`** is a macro that expands to a `match` returning `true` for the
  given pattern and `false` otherwise. It exists so you do not write five lines
  to ask a one-line question.
- **`#[must_use]`** makes the compiler warn if you call this and throw the
  answer away. On a function whose entire job is to return an answer, ignoring
  it is always a bug.

The method is three lines. The design is that it exists *at all* — as one rule,
on the type, in the journal that both surfaces already depend on, rather than as
an `if` in the window and another `if` in the CLI.

### The part that made it fatal rather than annoying

Worth recording because the measurement was not wrong. The governor (slice 4g)
narrows the number of parallel workers when narrowing costs nothing, and on a
saturated link one worker genuinely moves as many bytes as four — so "narrower
is no slower" measured true every time, and it walked all the way down to one.

What bytes per second cannot see is that **a run of one has no redundancy**. Any
single blocking thing — a prompt most obviously, but equally a slow rename or a
stalled socket — stops all of it rather than a quarter of it. So `Limits` gained
a `minimum` of two and narrowing stops there. An explicit `--parallel 1` is still
honoured, because *asking* for one is a different statement from *arriving* at one.

---

## 4. Reading a table by reflection

The storage feature — export, import, archive, and a reset that is not a cliff —
turns on one decision, and it is not the JSON.

The obvious way to export a table is a struct per table:

```rust
#[derive(Serialize)]
struct LinkRow { id: i64, name: String, source_root: String, /* ... */ }
```

This is wrong here, and the reason is in the module header
(`crates/tungstate-journal/src/storage.rs:13`):

> **The tables are read by reflection, not by a hand-written list of columns.**
> `SELECT *` plus the statement's own column names means a column added in some
> later slice is exported and imported without anyone remembering to come back
> here. The alternative is a struct per table that silently drops whatever it
> has not been taught about, and the day you find out is the day you needed the
> archive.

A hand-written struct is a list of columns that must be kept in step with the
schema by memory. Six migrations in, the odds of that holding are not good — and
the failure is silent, and surfaces on the worst possible day.

So `dump` asks the database what its own columns are:

```rust
fn dump(conn: &SqliteConnection, table: &str) -> Result<Vec<Value>> {
    let mut statement = conn.prepare(&format!("SELECT * FROM {table}"))?;
    let columns: Vec<String> = statement.column_names()
        .into_iter().map(str::to_string).collect();

    let rows = statement.query_map([], |row| {
        let mut object = Map::new();
        for (index, name) in columns.iter().enumerate() {
            object.insert(name.clone(), to_json(row.get_ref(index)?));
        }
        Ok(Value::Object(object))
    })?;
    ...
```

`statement.column_names()` is the reflection: whatever `SELECT *` just returned,
in order. Add a column in migration v7 and it exports itself, and
`an_export_carries_every_column_the_database_has` asserts exactly that — so the
property is checked rather than hoped for.

Note `format!("SELECT * FROM {table}")` — string interpolation into SQL, which
is normally how you get a SQL-injection hole. It is safe here for a reason worth
naming out loud: `table` comes from a private `const TABLES: [&str; 4]` in this
file, never from a caller. Row *values* still go through `?1`-style bound
parameters. The rule is that identifiers cannot be bound and must therefore come
from a fixed list; data can be bound and always is.

`load` is the mirror image, and its asymmetry is deliberate:

```rust
/// Insert `rows` into `table`, keeping only the columns this schema has.
///
/// A column the document carries and this build does not know is skipped
/// rather than refused: that is an archive from a *newer* tungstate, and
/// losing one field beats losing the whole restore.
```

That is the opposite of the rule for the journal file itself, where opening a
newer schema is **refused** (`JournalError::TooNew`, slice 4b). The two are not
inconsistent — they are answering different questions. A journal is the live
record of where your files went, so a build that might misread it must not touch
it. An archive is a document you are trying to recover from, so getting most of
it back beats getting none.

### Two version numbers

```rust
/// The document format's own version, which is not the database's.
///
/// Two numbers because they answer different questions. `schema` says what the
/// journal looked like; this says how to read the document describing it.
pub const EXPORT_VERSION: u32 = 1;
```

Same discipline as section 1, one level up. One number would have had to answer
both questions, and would have answered neither.

### The bug this nearly shipped with

Archive names were second-resolution, and a restore archives the current journal
immediately after reading the one it is restoring — so the second write landed on
top of the first. Caught by a test asserting the two names differ. Names are now
never reused. A one-second window is exactly the size of window that never shows
up in manual testing and always shows up in use.

---

## 5. Three bugs that compiled, type-checked, and passed

Everything above shipped green: `cargo test`, `clippy -D warnings`, `npm run
build`, CI on three platforms. All three of these were visible within a minute of
opening the window, and the window had not been opened.

### A `v-if` in the middle of a `v-if` / `v-else-if` chain

Vue's conditional rendering chains like an `if` / `else if` in any language, but
the chain is made of *attributes on adjacent elements*:

```html
<StorageView     v-else-if="tab === 'storage'" />
<LinksView       v-else-if="tab === 'links'" />
<ConnectionsView v-else-if="tab === 'connections'" />
<ActivityView    v-else />
```

The saved-link preview modal was inserted between `StorageView` and `LinksView`
carrying a plain `v-if`. That silently started a **new** chain: `LinksView` and
`ConnectionsView` became the `else-if` branches of *the modal*, not of the
onboarding branch. Both then rendered underneath the welcome panel rather than
instead of it.

Vue cannot warn about this, because a `v-if` is a perfectly legal neighbour for a
`v-else-if` — that is how you start a second chain on purpose. The fix moves the
overlays after the tab chain, beside the other modals, where the existing ones
already were.

The transferable lesson: **when adjacency is the syntax, inserting a line is a
structural edit.** Same family as inserting a line into a Rust `match` between
two arms that fall through to one body, or into a shell `case`.

### A field with no `type` attribute is not `input[type="text"]`

```css
/* before */
.opt select, .opt input[type="text"], .opt input[type="password"] { ... }
```

An `<input>` written without a `type` is a text field in every browser — and
matches neither selector. So the Find screen drew a white system-default box on
a dark sheet, and so did three fields in the add-a-link dialog. Nothing failed;
CSS has no such thing as a selector that did not match.

```css
/* after: by exclusion rather than by listing the types that count */
.opt select,
.opt input:not([type="checkbox"]):not([type="radio"]) { ... }
```

The rule is the interesting bit. The first form is a **whitelist that must be
updated at every call site that invents a new field**; the second is a
**blacklist of the two types that must not look like text boxes**, which is a
closed set. Slice 4e fixed exactly this for password fields, listed one more
type, and did not generalise — which is why it came back four slices later. When
a fix is "add one more to the list", ask whether the list is the bug.

### `.opt` is a grid that stretched its rows

```css
.opt { display: grid; align-content: start; gap: 5px; }
```

Two `.opt` boxes side by side in a `.pair` stretch to the height of the taller
one. A CSS grid's default `align-content` distributes spare height *between the
rows* — so in the shorter column, the gap between label and field grew and the
dropdown sat lower than its neighbour. `align-content: start` packs the rows at
the top and lets the spare height fall to the bottom where nobody looks.

### The lesson, restated

Slice 4d already recorded that *"compiles and launches is not a done bar for
anything with a screen."* Slice 5b did not even clear the launching half. All
three were found by running the app against a throwaway journal
(`TUNGSTATE_JOURNAL`, the hook slice 4e added for exactly this) and reading
screenshots tab by tab. That is now part of the bar, and it is written into
`docs/SYLLABUS.md` so the next slice inherits it.

---

## What this slice did not do

`explain` and `policy validate` have no screen — that is slice 5c, and the
argument for waiting is in the brief: a policy is a decision nothing acts on
until the planner (slice 6) and the executor (slice 7) exist, so a policy screen
designed now would be a window onto nothing. When it comes it is **read-only**;
the policy lives in the folder and is meant to be committed, so a window that
owns the file fights the text editor that also edits it.

`folder add` is still a stub on both surfaces, and stays one until slice 6 gives
a governed folder something to do.
