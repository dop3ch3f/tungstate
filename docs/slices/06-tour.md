# Tour: slice 6

The brief is in [06-planner.md](06-planner.md).

One new module of consequence (`tungstate-core::plan`), one small one that
carries the whole lesson (`tungstate-core::graph`), and a command that prints
what they decide. Six things worth slowing down for, and two of them are bugs
the tests found that no amount of reading would have.

---

## 1. Why the snapshot holds everything, when it only needs the verdict

A `Snapshot` is what one pass over a folder saw:

```rust
pub struct Snapshot {
    pub taken: Timestamp,
    pub entries: Vec<Attributes>,     // <- the interesting choice
    pub directories: BTreeSet<String>,
    pub case_sensitive: bool,
}
```

`Attributes` is the full description of a file — name, size, mime, EXIF map,
hash. The planner only ever asks it one question ("where does this belong?"),
so the obvious design stores the *answer* and throws the rest away. A folder of
200,000 photos would then cost a path and a destination each, rather than a
`BTreeMap` of EXIF tags each.

Keeping the whole thing is the load-bearing decision of the slice, and the
reason is one line in a test:

```rust
let after = plan.apply_to(&snap)?;
let again = policy.plan(&after);
prop_assert!(again.ops.is_empty());
```

*Apply the plan on paper, then plan again, and there should be nothing to do.*
That is the invariant DESIGN §1 says to write on the wall — **reconciliation is
idempotent and convergent** — and it is the property that makes "run this
continuously" a safe thing to offer.

To plan *again*, you have to classify the moved files again. To classify them,
you need their attributes. Store only the verdict and the second plan needs a
real filesystem with the moves already made — which means the invariant can
only be tested end-to-end, slowly, after slice 7 exists. Store the attributes
and it is three lines of pure data with no filesystem anywhere.

`Attributes::relocate` is the other half:

```rust
pub fn relocate(&mut self, relative_path: &str) {
    let (parent, name) = split(relative_path);
    self.parent = parent;
    self.name = name;
}
```

Name and parent follow the move; mime, EXIF, size and hash do not, because none
of them change when a file is renamed. That is a *domain* fact expressed as
code, and it is what makes the paper-apply honest rather than a convenient
fiction.

**`&mut self`** here is Rust's "I am going to change this". A method taking
`&self` may only read; one taking `&mut self` may write, and the compiler
guarantees nobody else holds a reference to the same value while it does. That
is the borrow checker doing the thing it exists for: not preventing mutation,
but preventing mutation *someone else can see half-done*.

The cost is real and is written down: a very large tree is held in memory. The
brief defers that to the SQLite index in slice 8 or 9, where an incremental
reader makes a cache worth its invalidation.

---

## 2. Four operations, and the one that is missing on purpose

DESIGN §4 lists nine op types. The planner ships four:

```rust
pub enum Op {
    MkDir { path: String },
    Move { from: String, to: String, because: Because },
    Quarantine { from: String, to: String, because: Parked },
    RmDir { path: String },
}
```

`Rename` folded into `Move`, because a rename is a move whose parent did not
change — two variants would be two identical match arms for ever, and which
*word* to print is a rendering decision the CLI makes by comparing parents.

The interesting absence is **`Trash`**. DESIGN §5 said `on_conflict =
"replace"` sends the existing file to the trash. The drain had already decided
otherwise, four slices earlier, and argued it in a comment:

> The existing file is quarantined rather than deleted. Replace is the user's
> decision about which copy they want, not permission to destroy the other one.

`--on-conflict replace` on a link and `on_conflict = "replace"` in a policy are
the same word, offered to the same person, about the same situation. If they
meant different things the product would be lying with one of them. So the
planner took the drain's reading and DESIGN was amended.

The dividend is out of proportion to the change. With no `Trash` and no
`Delete`, **nothing a slice-6 plan can express removes content** — so the second
invariant, *no content is ever lost*, stops being something to check carefully
and becomes something that is true by construction:

```rust
prop_assert_eq!(before_sizes, surviving_sizes);
```

A plan-only slice also never has to reason about the OS trash, remote
`.tungstate-trash/`, or retention policy. Deciding one word carefully deleted a
whole category of work.

**`Noop` is not an op either**, and the reason is structural rather than
aesthetic. An op is a node in a graph of state changes. A no-op has no edges,
no effect, and never goes away — so a plan containing them is *never empty*, and
"apply it and replan and get an empty plan" cannot even be stated. Reasons live
in a separate list:

```rust
pub struct Untouched { pub file: String, pub reason: Reason }
```

When a type makes your central invariant unstatable, that is the type telling
you something.

---

## 3. The graph, and why Kahn rather than depth-first

Ordering matters. Moving `a` to `b` when `b` is occupied by `c` moving to `d`
needs `c` to go first. The five edge rules, where `u -> v` reads *u before v*:

1. `MkDir(a/b) -> MkDir(a/b/c)` — a parent before its child.
2. `MkDir(parent(to)) -> Move(to)` — the directory exists before the file lands.
3. `Op{from == p} -> Move{to == p}` — **vacate before occupy.**
4. `Op{from == d} -> MkDir(d)` — a file moves aside before a directory takes its name.
5. `Op{from under d} -> RmDir(d)` — everything leaves before the directory goes.

Then a topological sort. The classic two algorithms are depth-first search and
Kahn's, and the choice is not a matter of taste here:

```rust
let mut ready: BTreeSet<usize> = (0..self.nodes).filter(|n| waiting[*n] == 0).collect();
let mut order = Vec::with_capacity(self.nodes);
while let Some(&node) = ready.iter().next() {
    ready.remove(&node);
    order.push(node);
    for &dependent in self.after.get(&node).into_iter().flatten() {
        waiting[dependent] -= 1;
        if waiting[dependent] == 0 { ready.insert(dependent); }
    }
}
```

Kahn's works by repeatedly taking a node with nothing left to wait for. When it
runs out, **whatever is left is exactly the set of nodes caught in cycles** —
which is precisely what a caller that intends to *break* those cycles needs. A
depth-first sort finds one back edge and leaves you to reconstruct the knot;
`petgraph::algo::toposort` returns `Cycle(node)`, a single node, for the same
reason. A test says so directly:

```rust
#[test]
fn a_rotation_is_reported_whole_rather_than_one_edge_of_it() {
    let sorted = graph(3, &[(0, 1), (1, 2), (2, 0)]).sort();
    assert_eq!(sorted.cyclic, BTreeSet::from([0, 1, 2]));
}
```

No dependency was taken. The graph is forty lines, single-use, and it is the
thing this slice exists to teach — a crate would have hidden it and answered the
cycle question in the wrong shape anyway.

### The `BTreeSet` is not a detail

`ready` could be a `VecDeque`. It is a `BTreeSet` because a `BTreeSet` is
*ordered*: when several nodes are simultaneously ready, the smallest index wins,
every time, regardless of the order edges happened to be added.

A `HashMap` or `HashSet` would have been the natural reach, and it would have
made the output vary between runs — Rust randomises hash seeds per process
precisely so you cannot accidentally depend on iteration order. There is no
`HashMap` anywhere in the planner, on purpose.

That pairs with one more line, in `Policy::plan`:

```rust
ops.sort_by(|a, b| a.ordering().cmp(&b.ordering()));
```

The ops are sorted into a canonical order *before* the graph is built, so a
node's index **is** its content order. Breaking a tie by index is therefore
breaking it by what the operation says, not by when it was constructed. That is
what makes "the same folder plans the same bytes" a property rather than a hope,
and it is asserted twice — once for two runs, once for two differently-ordered
inputs.

---

## 4. One temporary name unties a knot of any size

Two files that want to trade places are a cycle: `a -> b` and `b -> a`. The
filesystem has no atomic swap, so one of them has to step aside.

```rust
ops[victim] = Op::Move { from, to: swap, because: Because::MakeWay };
ops.push(Op::Move { from: swap, to, because });
```

Three choices in there worth naming.

**The temporary is a sibling of the destination**, `<to>.tungstate-swap`. That
directory provably exists — something is already sitting in it, which is the
entire reason there is a cycle — so no `MkDir` is needed, and a rename within
one directory is the cheapest and most certainly-atomic operation any backend
offers. A scratch directory would have to be created, cleaned up, and
crash-recovered.

**The victim is the lowest-indexed move in the cycle.** Since indices are
content order, "lowest" is a property of the folder rather than of the code, so
a given knot always breaks the same way.

**One temporary unties a rotation of any length.** A three-way rotation
`a->b->c->a` needs one, not three, and the test says exactly that. Watch it on a
real folder:

```
rename      a.txt -> b.txt.tungstate-swap  (out of the way, so two files can trade places)
rename      c.txt -> a.txt  (c-to-a)
rename      b.txt -> c.txt  (b-to-c)
rename      b.txt.tungstate-swap -> b.txt  (a-to-b)
```

Once `a` is out of the way the rest is an ordinary chain, and the chain is what
the topological sort was already good at. Breaking *one* edge converts the hard
problem into the easy one.

And there is a guard rather than an assertion:

```rust
for _ in 0..=ops.len() {
```

Each pass breaks one cycle; the bound means a bug cannot turn into a hang.
*Never loop forever* is worth more than a proof that you do not need to.

---

## 5. Two bugs the tests found, and what each one was really about

### The edge that pointed the wrong way

The swap test failed like this:

```
[("a.txt", "b.txt.tungstate-swap.tungstate-swap.tungstate-swap"), ...]
left: 3, right: 1
```

Three temporaries, each one wrapping the last. Rule 3 says *vacate before
occupy*: if some op moves a file **out of** the path you are moving **into**,
that op goes first. Correct — and it silently assumes the path is occupied
*right now*.

A swap's temporary name is not occupied. It is **produced** by the very op that
was just rewritten. So the edge pointed backwards, the graph looked cyclic
again, and the breaker invented a temporary for the temporary, once per pass.

The fix is to ask the question the rule was always really asking:

```rust
if existing.contains(to) {
    graph.require(other, index);   // there to vacate
} else {
    graph.require(index, other);   // not there: it has to be made first
}
```

`existing` is the snapshot's occupied set, threaded in. The lesson is not about
graphs: **a rule that reads as one thing and is implemented as another survives
every review until something outside its assumptions arrives.** "Something else
is moving off this path" and "this path is occupied" were the same sentence
until the planner started inventing paths.

### The move from a file to itself

```
second plan not empty, got [Move { from: "Text/one-1.txt", to: "Text/one-1.txt" }]
```

Two files both want `Text/one.txt`. The first takes it; the second is numbered
to `Text/one-1.txt`. Replan: `one-1.txt` still asks for `one.txt`, is refused
because the winner is sitting there, and tries `one-1.txt` — where it already
is.

The rule that lets it get that far is deliberate, and it is in the code:

> A candidate the mover is *itself* sitting on counts as free.

Without it, numbering walks `one-2`, `one-3`, `one-4` on every pass and the
folder never settles. With it, the file correctly resolves to where it stands —
and then the planner emitted a move from a path to itself, which is not work.
Recording it as `AlreadyThere` closes the loop.

Both bugs are the same shape: **the planner had started producing states the
first draft's rules had only ever been asked about consuming.**

---

## 6. The property that changed what the product does

The convergence test failed on a case nobody would have written by hand:

```
rename = "../{name}"
first:  a.txt -> absolute/.._a.txt
second: absolute/.._a.txt -> absolute/.._.._a.txt
```

The rename template reads `{name}` and adds to it. Once the file has been
renamed, `{name}` is different, so the template renders something new, so it
moves again — for ever. `rename = "copy-{name}"` is the version someone will
actually write.

This is not a planner bug. It is a **policy** that does not converge, and no
amount of reading the policy alone can tell you — convergence is a property of
the policy *and the folder together*.

The first instinct is to narrow the test's generator and note the hazard in the
docs. That is the wrong call, and the reason is slice 7. A dry run that churns
is harmless; the same policy under a daemon in `enforce` mode moves your files
for ever. So the product had to learn to detect it:

```rust
pub fn plan(&self, snap: &Snapshot) -> Plan {
    let mut plan = self.build(snap);
    if let Ok(after) = plan.apply_to(snap) {
        let again = self.build(&after);
        plan.settles = again.ops.is_empty();
        plan.unsettled = again.ops.iter().filter_map(Op::source).map(String::from).collect();
    }
    plan
}
```

Plan it, carry it out on paper, plan again. Two passes over a folder nothing has
been done to, which is a fair price for the only answer that exists. Note the
split into `build` and `plan`: the check calls `build`, not `plan`, or it would
call itself.

On a real folder:

```
This policy does not settle: carrying this out would leave 1 file(s) still
wanting to move, copy-a.txt among them.
A `rename` that reads {name} and adds to it renders differently once the file
is renamed. Fix the rule before applying anything.
```

And it changed what the invariant can honestly claim. Not *the second plan is
empty* — a policy can defeat that — but:

```rust
prop_assert_eq!(again.ops.is_empty(), plan.settles);
```

*The folder settles, or the plan said it would not.* Churning **silently** is
the outcome ruled out. That is a weaker theorem about the planner and a much
stronger promise about the product, and it exists because a property test was
allowed to generate a policy no human would.

---

## 7. What `apply_to` is really for

It looks like test scaffolding. It is the specification:

```rust
pub fn apply_to(&self, before: &Snapshot) -> Result<Snapshot, PlanError>
```

Every op checks its own precondition as it is reached — destination still
occupied, parent not made yet, directory not empty. So the fourth invariant is
not "the graph has no cycles", which tests the sort against itself, but:

```rust
prop_assert!(plan.apply_to(&snap).is_ok());
```

*Replaying this order against a folder never reaches an operation that cannot
run.* Same cost, and it tests the ordering against what the ordering is **for**.

It has three more lives after this slice. It is what slice 7's executor gets
checked against — run the real thing on a temp directory, survey again, compare
with what this predicted. It is how `apply` will detect a stale plan (DESIGN
§4: *stale plan → replan, not stale action*). And it is what `tungstate policy
diff` will be.

---

## 8. The shape of the command

`tungstate plan` walks up to the nearest `.tungstate/policy.toml` exactly as
`explain` does — the same function, moved to `cli::folder` so there is one
answer to where a folder begins rather than two.

It prints every op with the rule that decided it, every untouched file with its
reason, a blast radius, and `Nothing has been changed.`

**It does not write the plan it prints.** `tungstate plan --json > plan.json` is
the save. A read-only command that writes a file by default is a contradiction,
and staying provably harmless buys a test worth having: `plan_changes_nothing`
takes a census of the folder — every path, size and modification time — before
and after, and asserts they are identical.

Two things were found by *running* it rather than testing it, which is a lesson
this project keeps having to relearn:

- An ignored `.DS_Store` reported *"written too recently; 29s of cooldown
  left"*. The cooldown was checked before the decision, so a file that was never
  going to move was told to wait. It is asked after the decision now.
- An opaque `Photos.photoslibrary` appeared **nowhere at all**. The walk
  correctly refuses to enter it and directories were skipped without comment, so
  *"tungstate did not look in there"* and *"there is nothing in there"* read
  identically.

Neither was catchable by any test that was going to be written, because both are
about what the output *says*, and both took under a minute to see.

---

## What is next

Slice 7 executes a plan: `apply`, the circuit breaker's actual refusal, and
`undo` through the journal. Everything here is a dry run, which is the point —
the list you read and approve comes before anything moves.
