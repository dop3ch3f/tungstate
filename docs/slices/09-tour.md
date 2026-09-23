# Slice 9 tour: the folder keeps itself in order

Save a download into a governed folder, wait a few seconds, and find it where
the rules say it belongs. Nothing pressed.

Brief: [09-the-watcher.md](09-the-watcher.md). Folder sync
([9b–9e](09b-folder-sync.md)) is the same machinery with a different trigger.

---

## 1. Settling was already written

The hard-sounding half of a watcher is *wait until the file stops changing*.
It needed no code. `[defaults].cooldown` has been in the policy since slice 5,
and the planner already holds back anything written more recently than that
with `Reason::TooRecent`.

So the watcher never decides a file has settled. It notes that a folder was
stirred and comes back once the cooldown has passed, and the planner decides
the same way it does when somebody presses Tidy up. A file still being written
keeps producing events, and each one pushes the folder's deadline out:

```rust
pub fn stirred(&mut self, root: &str, now: Instant, cooldown: Duration) {
    self.due.insert(root.to_string(), now + cooldown);
}
```

`insert` into a `BTreeMap` replaces the old deadline for that key, so "push it
out" is one line. `Schedule` holds no handles and no clock of its own, and
`now` is passed in. That is what lets the deadline arithmetic be tested with
made-up instants instead of by sleeping.

## 2. The sweep is the truth

Watchers drop events. inotify overflows its queue, FSEvents coalesces, and a
file that arrives while the laptop sleeps may produce nothing at all. A watcher
you believe is a folder that quietly stops being tidied.

So an hourly sweep looks at every folder whether or not anything was heard.
The events only make the common case fast, and a bug in the event handling
now costs up to an hour's delay rather than a file left in the wrong place.

The loop is one thread blocking on a channel with a deadline:

```rust
let wait = schedule
    .quiet_for(now)
    .unwrap_or(sweep_every)
    .min(next_sweep.saturating_duration_since(now))
    .min(TICK);

if let Ok(batch) = events.recv_timeout(wait) { ... }
```

`recv_timeout` returns either a batch or a timeout, and both lead to the same
check: is any folder due, and is the sweep due. `TICK` caps the wait at a
quarter of a second so a stop is answered promptly. `saturating_duration_since`
returns zero rather than panicking when the deadline has already passed.

When the platform admits it lost something, the loop no longer waits the hour.
An error batch, or an FSEvents event that says *must rescan*, stirs every
folder under the path it names, or all of them if it names none:

```rust
if event.need_rescan() {
    stir_all(&event.paths, &resolved, now, &mut schedule);
    continue;
}
```

## 3. Its own writes are not news

Every move comes back as an event about a file that just appeared. Left alone,
that marks the folder dirty and buys another full survey after every action.
A tidy converges, so the second survey finds nothing and the loop ends. That
makes it a cost problem, and on a large folder the cost is the difference
between usable and not.

`Echoes` remembers the paths a plan wrote for ten seconds, both ends of every
move, and drops matching events. The entries age out in `forget_old`, so the
set cannot grow for ever in a process that runs for days.

That covered the moves. It did not cover the two things section 7 found.

## 4. `/private/tmp`, the third time

The path in an event is the one the platform resolved, not the one we asked to
watch. On macOS `/tmp` is a symlink, so a folder governed as `/tmp/watch9` is
reported as `/private/tmp/watch9`, and a root spelled the first way matches no
event at all. The journal hit this in 8b and the duplicate scan hit it after.

The first draft used bare `canonicalize`, which on Windows answers
`\\?\C:\media` while `notify` reports `C:\media`. The project already had a
fix: `tungstate_journal::resolve_for_lookup` takes that prefix off, and the
command line's folder filter now uses it too.

**Then Windows CI failed anyway**, and the reason is that the platforms
disagree about which spelling an event uses. FSEvents reports the resolved
path. Windows reports the path exactly as it was watched, and the CI runner's
temp folder is `C:\Users\RUNNER~1\...`, an old-style short name that
resolving turns into the long one. So resolving fixed macOS and broke Windows.
Neither spelling can be preferred, which is the same lesson the journal
learned in 8b. So each root is now matched under both:

```rust
fn spellings(root: &Path) -> Vec<String> {
    let given = root.to_string_lossy().to_string();
    let resolved = tungstate_journal::resolve_for_lookup(root)...;
    if resolved == given { vec![given] } else { vec![given, resolved] }
}
```

Both spellings lead to one folder, and the schedule is keyed by the folder
rather than by whichever spelling stirred it, so two names never mean two
surveys. The echo set expects every write under both spellings too. A test
recreates it on Unix with a symlink, which gives one folder two names the
same way the short name does.

Nothing would have failed loudly without that CI run. Every folder on
Windows would have been tidied an hour late, by the sweep.

## 5. A share is swept, not watched

FSEvents and inotify say nothing about a change another machine made to an SMB
share. So a folder on a share is not watched at all, and the first line says
which folders are in which group:

```
watching 2 folder(s), checking 1 on the hour
  the ones on a share are checked rather than watched: ...
```

The two are different promises, and the window and the terminal both say which
one they are making.

## 6. The shape

A new crate, `tungstate-watch`, holding the loop and nothing else. The deciding
stays in `tungstate-core` and the moving in `tungstate-execute`, so a watched
tidy is the same plan through the same executor as a pressed one. It lands in
History and comes back with `undo`.

`watch` takes `told: &mut dyn FnMut(&Noticed)`: the command line prints, the
window emits `watch://noticed`, a test pushes onto a channel. This is the same
seam the duplicate scan uses.

It moves files only in `enforce`. `observe` and `suggest` folders are counted
and reported, and never touched. There is one comment in `look()` saying so,
because that branch is the only place this slice can move a file.

## 7. Driving the window found five things

Every test was green before anybody drove the window. Driving it found these:

1. **The same "2 files arrived, waiting for you" line stacked once per
   sweep.** Waiting and trouble describe a state that persists, not a
   one-time event, so a new one now replaces the last one for that folder.
   The window reads the list back from the engine rather than keeping its
   own copy, so the rule lives in one place.
2. **History said nothing had happened**, under a panel saying a file had
   just been filed. Home now refetches History when the watcher notices
   something.
3. **A tidy reported what the plan meant to move, not what moved**, and did
   not report failures at all. It now counts files that actually moved (a
   refused `MkDir` is not a file) and reports the ones that could not be
   moved.
4. **Turning the switch on did nothing to files already waiting**, and nor
   did a launch, because nothing happened until an event arrived. `watch` now
   sweeps once before listening.
5. **Fixing the fourth broke the real-watcher test**, and working out why
   found the worst one. The test failure itself was the test's fault: the
   opening sweep now reports first, and the test took that report as the
   dropped file being filed and stopped listening. The event trace showed
   something worse, though. **Every look writes two `.tungstate-probe-*`
   files** to learn what the filesystem can do, and creating them changes the
   root directory's listing. Both came back as events, which woke the folder,
   which ran another look, which wrote more probes. A watched folder was
   surveyed about once a second for as long as it was watched. Probe names are
   now treated as tungstate's own in `is_ours`, and an event about the root
   itself is ignored: it only says the listing changed, and the child's own
   event already said that.

The fifth one was also why the real-watcher test had been passing: the probe
events woke the folder, and the dropped file got filed along with them. With
the loop gone, only the dropped file's own event can wake the folder, so the
test now proves the event was heard. It passed ten runs in a row on macOS,
and its first Windows run found section 4's second half. It also used to give
up after five quiet seconds, which is not generous on a busy runner; only the
30 second deadline ends the wait now.

## 8. What was checked by hand

On a throwaway journal, with `auto` set to enforce and `manual` to suggest, in
the embedded build:

- a file dropped before launch was filed by the opening sweep;
- a file dropped while watching was filed after 5 seconds (1 s debounce plus
  the folder's 3 s cooldown);
- switched off, a dropped file stayed where it was for 10 seconds;
- switched back on, the waiting file was filed in under a second;
- Home showed one waiting line for `manual`, and History agreed with the
  watcher panel;
- idle, the window used 0% CPU, which is the loop in section 7 gone.

## 9. What is checked

- **20 tests in `tungstate-watch`.** The deadline arithmetic, the echo set, the
  root mapping and the rescan stirring are pure. What a look decides is tested
  against real folders through `sweep`, which runs the same code an event does.
  One test uses the real platform watcher, with generous timeouts.
- Probe files and the root's own event are tested as not news, so the loop in
  section 7 cannot quietly come back. A root with two names is tested to wake
  its one folder once.
- `cargo fmt`, `clippy -D warnings`, 560 tests in parallel and again with
  `--test-threads=1`, `npm run build`, and CI on Linux, macOS and Windows.

## What is still not verified

- **Linux and Windows watcher behaviour is only checked by CI.** The real
  watcher test runs there, but nobody has dropped a file into a watched folder
  on either by hand.
- **No share has been watched.** That a folder on SMB is detected as networked
  and reported as "checked on the hour" is written and not yet seen on a real
  NAS.
- **`max_user_watches` has not been hit.** The code reports a folder that
  cannot be watched as trouble and keeps sweeping it, and nothing has made
  that happen.
- **A file that keeps growing** was not tried by hand. The cooldown that holds
  it back is the planner's and has its own tests from slice 6, but this is the
  first time that number matters with nobody watching.
- **Each look still probes the filesystem**, writing two small files it then
  deletes. They are ignored now, but a folder watched for a week gets probed
  on every look. Keeping one backend per folder would cache the answer.
