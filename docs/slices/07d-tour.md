# Slice 7d tour: filing defaults worth having, and trying them on

Three new starting layouts, and a command that shows what each of them would do
to *your* folder rather than describing what it does in general.

---

## 1. A sentence is not an answer

The layout picker slice 7b shipped offers four cards with a sentence each:

> **Sort by what each file is** — Images, video, documents, archives and
> installers each get a directory.

That is a description of a shape. The question somebody actually has is *what
will this do to my files*, and the two are not the same question — only the
second one has an answer they can check before agreeing to it.

`tungstate folder compare` answers the second one:

```
folder at /private/tmp/ts-dl
  7 file(s), walked once and shown against each layout

  (the rules you have)   0 of 7 would move, 0 dir(s) made, 0 removed
                           nothing would change
  downloads              7 of 7 would move, 3 dir(s) made, 14 removed
                           e.g. 2024/WhatsApp/Photo/jpg/IMG-…jpg → Images/IMG-…jpg
  media                  6 of 7 would move, 8 dir(s) made, 0 removed
                           e.g. 2024/WhatsApp/Photo/jpg/IMG-…jpg → 2024/WhatsApp/Photo/jpg/under-100MB/IMG-…jpg
  by-source              5 of 7 would move, 3 dir(s) made, 14 removed
                           e.g. 2024/WhatsApp/Photo/jpg/IMG-…jpg → WhatsApp/IMG-…jpg
```

Two things in that output are worth more than the counts.

**"The rules you have" is the first row.** Without it every other line is a
change from nothing; with it, every line is a change from where you actually
are. It is also the row that answers the question the 7b picker could not ask
at all, because that picker only appears for a folder with no rules.

**`removed` is a column.** `downloads` would remove fourteen directories from
that folder — it is not adding a shape, it is replacing one. That is the number
that would have warned somebody before slice 7b's walkthrough watched the
`documents` layout flatten a hand-made hierarchy and `rmdir` the remains.

## 2. Walked once

Seven layouts against one folder could mean seven walks, and at `meta` tier
that is seven times 64 KiB per file. It is one walk.

This is safe for a reason worth writing down, because the obvious worry is
real: each layout has its own `ignore` list, so a shared walk sounds like it
would let one layout's ignores leak into another's answer. It does not, because
`classify` checks `[folder].ignore` when it **plans**, not only when it walks
(`classify.rs`, the checks that come before any rule). A file the walk included
is still ignored by the layout that wanted it ignored.

412 files against 7 layouts: about a tenth of a second.

## 3. The three new layouts

### `media` — the five-level shape

`YEAR / APP / TYPE / EXTENSION / SIZE-BAND`, which slice 7c established is
expressible and converges. The app is read from the name the app gave the file,
because that is the one property of a file that does not change when the file
moves.

The ordering inside it is load-bearing and is asserted rather than assumed:
`IMG-20240312-WA0001.jpg` matches the camera rule as well as the WhatsApp one,
and only "first rule wins" decides which. There is a test named for exactly
that.

The fallback is called `Camera` rather than `Other`, on the grounds that a
directory called Other is where things go to be forgotten.

### `by-date` — year then month

One rule, `{date:%Y}/{date:%m}`, matching `glob = ["**"]`. The plainest shape
there is and the one most people already have in their heads.

The test for it asserts something narrow on purpose: that a file at the **top**
of the folder is filed. `**` reaching a root-level path is the one place a glob
of that shape is easy to get wrong, and `learn`'s own no-shape policy emits the
same pattern — so that assertion covers two callers.

### `by-source` — one directory per app

WhatsApp, Telegram, Screenshots, Camera. Nothing below. For a downloads folder,
where which app put it there matters more than when.

No inbox, deliberately. A file it does not recognise is left where it is rather
than swept into a directory named for the tool's own uncertainty — the same
decision `learn` made in 7c and for the same reason.

## 4. What the property tests already demanded

The four starter-layout tests slice 7b left behind are a good bar and the new
layouts had to clear it unchanged: parse without warnings, have rules, describe
themselves in under 46 characters, settle, and — the strong one — be applied on
paper and leave nothing further to do.

They were not, however, enough. The snapshot those tests share is built by a
helper that leaves `mime` and `mtime` unset, which is right for layouts that
route by extension and useless for layouts that route by what a file *is*:
against that folder `media` matches nothing at all and settles **vacuously**.
Four green tests, zero coverage.

So `media_folder()` is a second fixture with real media types and real dates —
a WhatsApp photo, a WhatsApp video, a Telegram photo, a screenshot, a camera
photo and a text file — and the new tests plan against that. The lesson is the
one this project keeps relearning in a new costume: a passing test proves
something about the fixture first and the code second.

## 5. Verified

458 tests, fmt and clippy clean. New:

- `the_media_layout_files_five_levels_deep_and_knows_which_app_sent_it`
- `the_media_layout_settles_on_a_folder_it_has_already_filed`
- `by_source_puts_a_whatsapp_photo_under_whatsapp_and_not_under_camera`
- `by_date_files_everything_it_can_date_and_leaves_the_rest`
- `compare_shows_every_layout_against_one_folder_and_changes_nothing`, which
  also asserts the folder is untouched afterwards and that a folder with rules
  gets `(the rules you have)` as its first row.

By hand: `compare` against a folder of loose downloads, against a folder
already filed five levels deep, and against 412 photos in one directory.

**Not verified:** the window, which still shows four cards and still only shows
them for a folder with no rules. That is 7e.
