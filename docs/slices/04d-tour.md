# Slice 4d tour: the window and the run

Read this next to the diff. This slice was not on the roadmap. It exists
because the first real use of the desktop app found three bugs in one
session, and the third one was costing gigabytes silently.

Files: `tungstate-journal/src/links.rs` gained `interrupted` and `link_by_id`,
`tungstate-transfer/src/lib.rs` gained `discard` and lost a method to a free
function, `tungstate-gui` gained three commands and a window-close handler, and
the CLI gained `link unfinished` and `link discard`.

---

## Part 0: Three bugs, three different kinds

Worth separating, because they want different habits.

**An empty ACL.** Tauri 2 resolves a permission list from `capabilities/`, and
there was no such directory, so the list was the two bytes `{}`. Every core
command the frontend called was denied — including `event.listen`, which the
window uses to hear about progress. The tell was sitting in the build output at
`target/*/build/tungstate-gui-*/out/capabilities.json` the whole time.

*Habit: when a framework generates something, read what it generated.*

**Class names with no CSS.** The Activity view used `lede`, `act`, `field`,
`size` and `empty`, none of which the stylesheet defines, and wrapped its
search form in `row`, which is the dual-pane *file row*. It also lacked the
`.sheet` wrapper every other view has — the flex child that takes the
remaining height and scrolls — so the page had no padding and a long history
could not be scrolled at all.

*Habit: a cross-check between two files nobody diffs together is worth
automating. It is a dozen lines of Python and it now runs over every
component.*

**An unreachable interrupted run.** This one is the slice.

## Part 1: The bug that was worth a slice

Three true things combined into something none of them implies.

1. A one-off browser transfer creates a link with `saved = 0`.
2. `Journal::links` returns `WHERE saved = 1`, so it never appears in the UI.
3. Recovery is scoped per link: `incomplete_for_link(self.link.id)`.

Kill the app mid-file and the operation stays `intended` with a partial at the
destination. Select the same files again and you get a *new* link with a *new*
id, whose recovery sweep cannot see the old operation. The old row is
permanent. The partial is permanent. Observed for real: 2.9 GB, invisible, and
it would have grown by one file every time it happened.

Nothing here is a coding mistake. Each decision was right on its own, and the
hole is in the space between them. That is the kind of bug that only use finds,
which is the argument for using the thing you build.

## Part 2: `Drop` as a guarantee

```rust
struct RunGuard(AppHandle);

impl Drop for RunGuard {
    fn drop(&mut self) {
        let state = self.0.state::<App>();
        state.conflicts.close();
        state.running.store(false, Ordering::SeqCst);
        …
    }
}
```

and in the worker thread:

```rust
let _guard = RunGuard(worker.clone());
```

The old code set `running = false` on the last line of the thread. That is
correct on the happy path and wrong the moment anything panics: the flag stays
`true`, and every later transfer is refused with "a transfer is already
running" for the rest of the session, with no way back but restarting the app.

`Drop` runs while a thread unwinds from a panic, so the guard holds where the
last line does not. This is RAII, and it is the Rust answer to `finally`:
instead of remembering to clean up on every exit path, you make cleanup the
consequence of a value going out of scope.

Two details. The binding is `let _guard`, not `let _` — `let _` drops
*immediately*, which would release the flag before the work started. And the
guard owns an `AppHandle` rather than borrowing state, because it has to
outlive everything else in the thread.

## Part 3: One function for a rule that deletes things

`recover` and `discard` both have to clear what an interrupted operation left
behind. They could each have done it. They do not:

```rust
fn clear_partial(destination: &dyn Backend, op: &tungstate_journal::Op) {
    let Some(landing) = &op.destination else { return };

    if destination.capabilities().atomic_rename {
        let _ = destination.remove_file(&temp_name(&landing.path, op.id));
        return;
    }
    let _ = destination.remove_file(&landing.path);
    sweep_orphans(destination, &landing.path);
}
```

The reason to insist is the sweep. `sweep_orphans` deletes files matching
`<name>.<8 alphanumerics>` — a pattern narrow enough to spare `a.mp4.backup`
and wide enough to catch OpenDAL's random FTP temporaries. Two copies of a rule
like that drift, and the drift shows up as a file the user wanted, deleted.

Note it became a *free function* taking `&dyn Backend` rather than a method on
`Transfer`. `discard` has no `Transfer` — there is no run, no resolver, no
progress sink, just a link and a destination. Making the helper depend only on
what it actually uses is what let it be shared at all.

## Part 4: The join that beat a schema change

I said in the brief that this needed no migration, and that is worth showing:

```rust
pub fn interrupted(&self) -> Result<Vec<Interrupted>> {
    let mut by_link: BTreeMap<i64, Vec<Op>> = BTreeMap::new();
    for op in self.incomplete()? {
        if let Some(id) = op.link_id {
            by_link.entry(id.0).or_default().push(op);
        }
    }
    …
}
```

`Journal::incomplete` was already global — it has been since slice 2 — and the
`ops.link_id` column has existed since v2. The only thing missing was that `Op`
did not expose it; `row_to_op` read every other column and skipped that one. So
the change is one field on a struct and one line in a row mapper.

`BTreeMap` rather than `HashMap` on purpose: it iterates in key order, so the
list comes out in link-creation order every time. A test that asserts on
ordering must not depend on a hash seed.

And note the error handling:

```rust
runs.push(Interrupted { link: self.link_by_id(LinkId(id))?, ops });
```

The `?` propagates rather than skipping a link that cannot be read. The foreign
key makes that unreachable, and the temptation with unreachable cases is to
swallow them — but a swallowed error here returns a *shorter list than the
truth* to a screen whose whole job is saying what was left unfinished. Silently
under-reporting is worse than failing.

## Part 5: The window lifecycle, and a thing built then backed out

This slice first made closing the window *hide* it, so a drain could carry on
invisibly:

```rust
.on_window_event(|window, event| {
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        if state.running.load(Ordering::SeqCst) {
            api.prevent_close();
            let _ = window.hide();
        }
    }
})
```

It worked, and it was removed. Worth recording why, because the code was fine
and the idea was not.

It buys a half-daemon. Windows and Linux have no dock icon, so once the window
is hidden there is no way to bring it back until the run ends — the app is
running, doing something, with no surface. Showing it again on completion
bounds that, but "the app is still going, somewhere" is a worse thing to be
uncertain about than "closing stops it".

So the final behaviour is the *default* behaviour, and the diff for it is a
comment explaining that the absence is deliberate:

```rust
// Deliberately no close handler. The window and the run are the same thing
// until slice 10 puts the engine in a daemon …
```

The thing that makes this acceptable is everything else in this slice. Closing
mid-drain leaves an `intended` operation and a partial; the next launch names
it and offers Resume or Clean up. **Making an interruption cheap is what earns
the right to make closing simple.** Had this slice only done the window half,
it would have been a worse app.

A real daemon is slice 10. Until then the honest statement is the one the
release notes now make: closing the window stops the drain, and nothing is
lost when it does.

## Part 6: Offering rather than doing

The banner has two buttons and no default action. That was a deliberate choice
against a tidier-sounding alternative — clean up automatically at startup so
orphans can never accumulate.

It was rejected because it throws away work. In the case that prompted the
slice there were 2.9 GB already copied; deleting them without asking, to save a
folder from clutter the user has not noticed, is the tool deciding something
that is not its to decide. Equally, resuming automatically would start a
multi-gigabyte network transfer while someone was reading a screen.

So: say what happened, say what each button does, and wait. The note under the
banner says the thing that actually matters — *nothing was lost, every original
is still where it was* — because a person who has just seen "3.3 GB left
unfinished" needs to know that first.

## What to look at in the diff

1. `tungstate-transfer/src/lib.rs` — `clear_partial`, `sweep_orphans`,
   `discard`, and how little `recover` has left.
2. `tungstate-journal/src/links.rs` — `interrupted`, and `Interrupted::bytes`
   with its comment about what that number is and is not.
3. `tungstate-gui/src/main.rs` — `RunGuard`, and the comment where a close
   handler is not.
4. `tungstate-transfer/src/tests.rs` — the bottom, especially
   `interrupted_work_is_found_even_on_a_link_the_saved_list_hides`, which is
   the bug written down as an assertion.
