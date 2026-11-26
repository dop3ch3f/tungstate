# Phone slice G tour: transfers that survive the screen, the background and the network

The user kept tapping their S24 so it would not sleep during a 200-file copy,
and was afraid to switch apps in case the copy was lost. The ask: beat ES File
Explorer at this. A copy keeps going with the screen off, in the background,
through Doze and through Wi-Fi dropping, and resumes properly after the OS
kills the app.

Decided with the user before building:

- A Move removes each original as soon as its copy is checked, even while
  locked. This replaces the 2026-10-25 rule that removals wait for the app.
- Only transfers the person started keep going. Nothing is scheduled, and
  there is no desktop daemon.

What changed, by layer:

1. **Engine** (`tungstate-transfer`):
   - an end that stops answering is told apart from one that changed;
   - a file cut off by it is left to resume, not marked failed;
   - a resumed one-off copy skips what it already sent;
   - a narrowed run no longer hangs when an end goes.
2. **App, Rust** (`tungstate-gui`):
   - a per-file ledger kept beside the events;
   - a keep-alive guard that drives the phone;
   - a worker that waits for a connection to come back;
   - `transfer_ledger`.
3. **Phone plugin**:
   - Android: a user-initiated data transfer job or a foreground service,
     wake and Wi-Fi locks, and the notification with Stop;
   - iOS: a continued processing task, or the grace period.
4. **Screens**:
   - catch up when shown again;
   - a "Waiting for Area51" bar;
   - a Resume card on Files home.

---

## 1. Silence is not a different destination

The engine checks before every file that the destination is still the
storage it started with (`root_token`). Until now, any failure there set
`destination_lost` and ended the run. That is right for an unmounted drive,
whose mount point comes back as an empty folder on the boot disk. On a phone
it also ended every copy at the first Wi-Fi drop.

`Summary.unreachable` now says *why* the destination was lost: it stopped
answering. It is always set **together with** `destination_lost`. Every
existing caller (desktop, CLI, Free up) still stops exactly as before. Only a
caller that knows about `unreachable` may wait.

Who may wait is the app's decision (`may_wait_for` in `lib.rs`): a phone,
whose own storage cannot unmount, or a desktop run whose destinations are all
connections. A desktop folder never waits.

The source counts too. A copy *down* from a NAS that loses Wi-Fi used to fail
every remaining file on its own. Now, when a file fails, the engine asks both
ends whether they still answer (`an_end_is_silent`). It asks only after a
failure, so a healthy run pays nothing.

## 2. A file cut off mid-write is left to resume

`copy_to` already left a file's journal operation open (`Intended`) when the
run was stopped. That is what puts it in front of the user with Resume and
Clean up. A file cut off by an end going quiet now gets the same treatment,
and `work()` reports it as `DestinationLost`, not as a failure.

Why it matters: an open operation has its half-written partial cleared by the
next run of the link (`recover()`). Marked failed, it stayed at the far side
for good. The emulator test left six of these on the server before the fix.

Test: `a_file_cut_off_by_the_network_is_left_to_resume_and_its_partial_cleared`.
It fails without the fix ("Resume is offered for it").

## 3. A resumed copy does not read everything again

Resuming used to mean re-running the link. For a copy, every file already
sent was found "already present" by reading **both** copies. That is 200
photos read back over Wi-Fi to resume after a drop.

`carried_already()` skips a file when all of these hold:

- the link is a one-off (`!link.saved`, which is what Copy here makes);
- the link is a copy (`SourcePolicy::Keep`);
- the journal has a committed transfer of that path at that size for this
  link (`Journal::landed`);
- the copy at the far side is still that size.

Such a file is reported as the new `FileOutcome::Carried` ("sent and checked
earlier in this copy, before it was interrupted").

Saved links are excluded: they run again days later, when the same size is no
proof of the same content. Moves are excluded too: their committed files are
gone from the source anyway.

The size check lists each destination folder once, not one question per file.
On the emulator's network, one round trip per file made resuming after 40
photos wait a minute before the first byte moved.

## 4. The hang the emulator found

With the NAS paused mid-copy, the transfer showed 60% for ever. A thread dump
(`debuggerd -b`) showed one engine worker parked in
`Governor::wait_for_room`. The run had narrowed from four files at a time to
three. The other workers left when they found the NAS gone, and nothing woke
the parked one.

Its wake-up test now counts a lost end as "over":

```rust
let empty = || lock(queue).is_empty() || self.cancelled() || lock(shared).destination_lost;
```

The test `a_narrowed_run_whose_destination_goes_quiet_still_ends` forces the
narrowing with one "too many connections" refusal, then goes silent. A hang
cannot fail a test on its own, so the test runs a watchdog thread that exits
the process after 30 s. Without the fix it fails every time; with it, it
passes.

## 5. Waiting for the connection (`lib.rs`)

The queue worker now loops per job:

```rust
let (total, failure) = loop {
    ledger.begin(job.id); emit("transfer://job", …);
    let (total, failure) = run_job(…);
    let waits = total.unreachable && may_wait_for(&job) && !cancel.asked();
    if !waits || !wait_until_back(…) { break (total, failure); }
};
```

`wait_until_back`:

- emits `transfer://waiting`;
- asks both ends every 5, 10, 20, then 30 seconds;
- sleeps in half-second steps, so Stop is heard at once;
- gives up after 30 minutes.

When both ends answer, the same job runs again. It is resumed from the
journal: carried files are skipped, and open operations have their partials
cleared and are sent again.

`loop { … break value }` is Rust's loop-as-expression: `break (total,
failure)` both leaves the loop and is the value the `let` receives.

## 6. Keeping the phone going (`keepalive.rs`, the plugin)

`Keepalive` is a guard. `begin` asks the phone to keep the run going and the
screen on, and `Drop` ends both. A run that panics still lets the phone sleep
and clears its notification, because `Drop` runs during unwinding.

It lives inside `EventProgress`, which every per-file event already passes
through, so the notification is fed from Rust. That matters because a phone
pauses a background web view's scripts, which is exactly when the
notification matters.

`pulse()` updates the notification at most once a second. The plugin's answer
to each update says whether Stop was pressed there. That is simpler than a
callback channel and cannot drop a message.

The notification's words are a pure function, `notice()`, tested on the Mac:
"Copying to Area51", "34 of 200 · IMG_2041.jpg", or "Waiting for Area51 to
answer…".

**Android** (`TransferKeeper.kt`):

- Android 14 and later: a *user-initiated data transfer* job, the API made for
  "a transfer the person started". It has no time limit; a data-sync service
  is capped at 6 hours a day from Android 15. The job holds no work, only the
  process.
- Before 14, or with no network (a copy to the card in airplane mode): a
  data-sync foreground service.
- Both: a partial wake lock, so the processor does not sleep with the screen,
  and a Wi-Fi lock.
- The notification is detached from the job. When Android stops the job
  because the network went, the shade still says "Waiting", and the job is
  asked for again (`onStopJob` returns true). It comes back with the network.
- Notifications are asked for once, at the first paste, which is when the
  reason is plain. Refused, the copy still runs; it is just unseen.

**iOS** (`PhotosPlugin.swift`, the plugin's first real iOS code):

- iOS 26: a `BGContinuedProcessingTaskRequest`, one identifier per run under a
  wildcard in `BGTaskSchedulerPermittedIdentifiers` (`Info.ios.plist`). The
  system shows its progress card. When it takes the time back, that becomes
  Stop after this file, and Resume is offered on return.
- Before 26: the grace period `beginBackgroundTask` gives, then the same Stop.
- `keepAwake`: `isIdleTimerDisabled`.

## 7. Coming back to the app

- `ledger.rs` keeps the running transfer's files in Rust, with the same rules
  as `useTransfer.ts`.
- `transfer_ledger` returns that ledger, whether the run is waiting, and how
  the last transfer ended (with its rows).
- `catchUp()` lays that over the window's ledger when the page becomes
  visible again.
- A copy the OS killed shows on Files home as "A copy to Area51 stopped before
  it finished", with **Resume** (`resume_interrupted`) and **Clean up**
  (`discard_interrupted`). These are the commands the desktop banner already
  used.

## Verified

- **Engine:** 102 tests, including silence, a silent source, a cut-off file,
  carried files, saved links, and the narrowed hang (each new regression test
  checked to fail without its fix).
- **App:** `ledger::` and `keepalive::` tests; `npm test`, `vue-tsc`,
  `check-css`; clippy and fmt.
- **Android emulator, release build, 200 files (263 MB) to Docker Samba:**
  - the notification prompt appears at the first paste;
  - the user-initiated job runs with `HAS_FOREGROUND_EXEMPTION`, and the
    notification shows "Copying to Docker · 23 of 200";
  - Home, screen off and forced deep Doze: files kept landing (35 → 50 in
    45 s);
  - network off (`svc wifi/data disable`): the engine noticed in about 31 s
    (the SMB request timeout), and the notification said "Waiting for Docker
    to answer";
  - network back on: "both ends answer again; carrying on" about 20 s later,
    screen still off;
  - all 200 files arrived, and every checksum matches the phone's.
- **Killed mid-copy** (`kill -9` at 40 files, as the system does):
  - on reopening, Files home showed the Resume card;
  - Resume skipped the 40 and sent the rest;
  - no partials were left on the server, and all 200 checksums match.

  That run found three things, now fixed: the slow skip check (section 3), the
  card's wrong count ("3 files left" counted the files mid-send), and the
  dead run's notification left in the shade, now cleared when the app starts
  (`TransferKeeper.clearStale`).
- **iOS:** the plugin's Swift compiles against the iOS 26 SDK, continued
  processing task included. It was compiled in a scratch package with
  Tauri's Swift API, because the full simulator build hung for an hour at
  Xcode's call back into the Tauri CLI, before reaching Swift. It has not run
  on a simulator or an iPhone.
- **Workspace:** `cargo test --workspace` passes except
  `tungstate-watch`'s folder-watching test, which fails on this Mac whatever
  the code (FSEvents) and runs in CI.
- **Not yet:** the user's S24.

## Known gaps

- `docker pause` (TCP stays open, the server stops reading) holds an SMB write
  until the server returns. It does not time out: smb-rs's 30 s limit covers
  replies, not a write the kernel cannot send. A real Wi-Fi drop errors
  instead, as tested. Files caught that way are re-sent; their partials
  predate the fix in section 2.
- A run killed *between* files has no open operation, so no Resume card
  appears. Its link is still in the journal, but `interrupted()` looks for
  open operations. Rare, because a kill almost always lands mid-file.
- A file that genuinely failed is not retried automatically within the run.
  "Copy the rest" re-sends it, skipping what was carried.
