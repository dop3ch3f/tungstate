# Phone slice F tour: a home tab, paste in any tab, colour, per-file progress

The user tried slice E on their S24 and asked for six things:

1. The home tab is taken over: choosing FTP on Files home turned that tab
   into FTP, and getting home again meant knowing to press +.
2. No sheet asking which tab to send to. Copy or Move should hold the files,
   and every tab, including another on the same storage, should offer
   "Copy here" or "Move here".
3. Tab icons, so two tabs showing a "Photos" on different places are told
   apart.
4. The copy screen showed only totals. With 200 files they couldn't tell
   which had gone, which was going and which was waiting, and the art didn't
   move.
5. Dark mode was too black. Shown four background colours, they said no:
   colour belongs on the elements, the way the desktop app does it.
6. Errors were bare red sentences.

This slice is UI only. No engine or Rust changes.

---

## 1. Home stays home (`state/useWindows.ts`)

Tab 0 is `HOME`. `close(HOME)` does nothing and the strip draws no ✕ on it.

Who decides what opens where is split in two:

- **Files home** knows which volume is the phone's own storage, so it decides.
  Internal storage, its kinds of file, and recent folders on it open in the
  tab you are in (`w.go`). A connection or a removable card goes through
  `w.openTab` (`elsewhere()` in `files/FilesHome.vue`).
- **`openTab`** keeps the tab rules, and stays pure enough to test without
  volumes:
  - in a blank tab made with +, the place opens in that tab, even when
    another tab is already there. Two tabs on one place is how you copy
    between two of its folders;
  - from home or a tab in use, it switches to the tab already in that place,
    else opens a new tab;
  - with all five open, the last tab gives way, never home.

`within()` now matches a folder on a connection as well as a whole one:
`Area51:Photos` holds `Area51:Photos/2026` but not `Area51:Media`. The rule is
`root.endsWith(":") ? root : root + "/"`, because a connection's name is
followed directly by its folders (`Area51:Photos`), with no `/` between.

`scripts/phone-windows.test.mjs` covers each rule, including that home never
closes and that the last tab gives way.

## 2. Copy and Move go straight to the bar (`files/FileWindow.vue`)

The `move` sheet, `others`, `sendToOther` and `pending` are gone. Copy and
Move call `startClip`, which already held the files for ES-style pasting.
The clipboard is module-level, so `ClipBar` shows in every tab. It still
greys out "Copy here" in the folder the files came from, and now says "Go to
another tab or folder to put them there".

## 3. Place marks (`lib/look.ts`, `screens/FilesScreen.vue`)

`Look` is a mark on a colour square: `HOME_LOOK`, `PHONE_LOOK`, `CARD_LOOK`,
and `connectionLook(scheme)`, one desktop section colour per kind of
connection (SMB purple, FTP teal, SFTP blue, WebDAV orange, S3 green).
`folderLook(path, volumes, schemeOf)` picks one for any folder.

Each tab shows its mark, the folder's name and, once you are inside a folder,
the place under it ("Camera / Internal storage"). `faces` is a `computed`
over the tabs, so the marks are worked out once per change, not per render.
Files home's connection chips and the Connections list use the same colours.

`topOf` became generic, `topOf<T extends Top>(path, tops: T[]): T | Top | null`.
It returns one of the tops it was given, so a caller passing volumes gets the
volume back, `removable` and all, and `"removable" in top` narrows it without
a cast.

## 4. Per-file progress (`files/MovingScreen.vue`, `lib/progress.ts`)

The ledger (`state/useTransfer.ts`) already kept every file's state and
bytes. The phone just never drew it. Now:

- Totals at the top, smaller, with the next block blinking.
- A list of every file in the order sent. Each line has a mark (waiting,
  sending, checking, done, already there, set aside, failed) and either a
  word or, while the file moves, the desktop's striped bar: blue while
  sending, green while checking it arrived. A failed line gives the
  engine's reason.
- Every line is 60px, bar or word. Which lines are on screen is then
  arithmetic (`rowsInView`, as the folder lists do), so a 10,000-photo Free
  up scrolls as freely as 20 files.
- The list follows the file being sent, one finished line above it, unless
  a finger touched the list in the last four seconds.
- The phone-to-NAS scene's three arrows light one after another
  (`PScene` `moving`). The collapsed bar fills as it goes and has stepping
  arrows. All motion stops under reduced motion.

A failed file's reason is not on its own `finished` event: the engine puts
it in the run's summary (`Summary.failures`). The screen looks each failed
line's reason up there (`reasonOf`), and the failure bar's Details lists
the first five. `PTrouble` takes a `detail` for this: the sentence comes
from the screen, the reasons from the engine.

`markOf` is a total map with a fallback, as `lib/tone.ts` is for the
desktop: a state the engine adds later gets a known mark, not an unstyled
line. `scripts/phone-progress.test.mjs` covers the words, marks and percent.

## 5. Colour on the elements

As on the desktop, colour sits in the section tiles and buttons, not the
background:

- the bottom tabs draw each mark on its section's colour square (`TabBar.vue`);
- a section's first screen has its square by the title (`PScreen` `tile`),
  as the desktop's windows do in their title bars;
- the window tabs and connection rows carry the place marks above.

The four background tints tried first (navy, berry, moss, warm brown) were
dropped at the user's call.

## 6. Errors as the desktop's failure bar (`ui/PTrouble.vue`)

The desktop's retro `Notice` failure tone, at phone size: a coral bar
(`--alarm-bar`) outlined in ink with the warning mark (`ui/AlertIcon.vue`)
and the sentence in ink. Details still opens the engine's words. Every
`class="line bad"` sentence in the phone screens is now a `PTrouble`, so a
screen never mixes a bar and a stray red line.

## Verified

- `npm test` (103), `vue-tsc`, `check-css`.
- Harness shots: home tab with two connection tabs, a tab with its place
  line, the copy bar in another tab, the copy screen part-way and ended with
  a failure, Connections.
- Android emulator, release build, against Docker Samba, by hand: Docker
  opened from home in its own tab with Home kept; Internal storage opened
  inside the home tab; two Download files ticked, Copy went straight to the
  bar with no sheet ("Go to another tab or folder…"); in the Docker tab,
  Copy here started the run; both files listed at once, one sending (blue
  stripes), one checking (green); ended "Copied and checked" / "Already
  there".
- Two earlier runs failed with `bad magic` errors from the SMB library and
  "destination is no longer the storage we started with". The emulator was
  overloaded (load average 29 while two builds ran on the Mac; Android's
  system process then stopped responding). After a cold boot the same copy
  passed, and the Mac passed the SMB end-to-end tests against the same
  server. Those runs did show the missing reasons, which is why failed lines
  now look them up in the summary.
- Not run here: the user's S24 against Area51.
