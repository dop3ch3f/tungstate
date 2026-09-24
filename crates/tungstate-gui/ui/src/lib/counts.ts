// A count of files is not a count of operations.
//
// Slice 7b shipped a screen that promised "5 of 6 files would move" and then
// reported **"moved 9 file(s)"**. `Applied::done` counts operations, and that
// plan also made three directories and removed one: 5 + 3 + 1 = 9. Somebody
// who read both numbers was told their folder had been rearranged nearly twice
// as much as it was. It survived 435 passing tests because nothing tested the
// sentence.
//
// `docs/SEAM.md` makes it safety property 3. This file makes it a type error.
//
// `FileCount` is a branded number that nothing outside this file can mint.
// There is deliberately no `asFileCount(n: number)`: the only way to obtain
// one is to hand over the engine value that genuinely is a file count. Write
// `moves.length` or `ops.length` where a `FileCount` is wanted and `vue-tsc`
// refuses it, on every platform CI runs on.

import type { Cleared, DupeRow, Found, Notice, Outcome, PastRun, PreviewView, PutBackDone, ScanProgress, TidyDone } from "../engine/types";

declare const isFileCount: unique symbol;

/** A number of files. Obtainable only from an engine field that counts files. */
export type FileCount = number & { readonly [isFileCount]: true };

const seal = (n: number) => n as FileCount;

/** Files a finished tidy actually moved. Not `TidyDone.moved + created`. */
export const moved = (done: TidyDone): FileCount => seal(done.moved);

/** Files a finished tidy left because they changed while we looked. */
export const skipped = (done: TidyDone): FileCount => seal(done.skipped);

/** Files a finished tidy could not move. */
export const failed = (done: TidyDone): FileCount => seal(done.failed);

/** Files an undo put back. Counted from the journal's renames for that plan,
 *  which is a different number from that plan's operations. */
export const putBack = (done: PutBackDone): FileCount => seal(done.files);

/** Files a preview says would move. `blast.files`, never `plan.ops.length`. */
export const wouldMove = (view: PreviewView): FileCount => seal(view.files);

/** Files the watcher filed, or found waiting. The engine counts what moved,
 *  not the plan's operations. */
export const noticed = (notice: Notice): FileCount => seal(notice.files);

/** Files a past tidy or clean-up moved. The journal counts committed file
 *  ops only, never directories or failures. */
export const ran = (run: PastRun): FileCount => seal(run.files);

/** Files a duplicate scan looked at. */
export const lookedAt = (found: Found): FileCount => seal(found.files);

/** Extra copies a scan found: files, and for a folder group every file in it. */
export const extraIn = (found: Found): FileCount => seal(found.extra_files);

/** Files a running scan has reached, and how many of those it opened. */
export const reached = (progress: ScanProgress): FileCount => seal(progress.looked);
export const opened = (progress: ScanProgress): FileCount => seal(progress.read);

/** Files a clean-up set aside or trashed. */
export const cleared = (done: Cleared): FileCount => seal(done.files);

/** Files each copy of a folder group holds. */
export const eachHolds = (row: DupeRow): FileCount => seal(row.files);

/** What `undo_duplicates` returns, which counts files put back. */
export const undidDuplicates = (files: number): FileCount => seal(files);

/** Files the ticks would deal with: a ticked file is one, a ticked folder is
 *  every file in it. Summed from engine counts, so still a count of files. */
export function tickedAcross(rows: DupeRow[], tickedIn: (row: DupeRow) => number): FileCount {
  let total = 0;
  for (const row of rows) {
    const count = tickedIn(row);
    total += row.folder ? count * row.files : count;
  }
  return seal(total);
}

/** Files a preview looked at. */
export const outOf = (view: PreviewView): FileCount => seal(view.of);

/** Files one way of filing would move. */
export const movedBy = (outcome: Outcome): FileCount => seal(outcome.files);

/** Files one way of filing looked at. */
export const seenBy = (outcome: Outcome): FileCount => seal(outcome.of);

/**
 * Directories, which are counted and said separately on purpose.
 *
 * Safety property 5: directories removed is the number that says a layout is
 * *replacing* a shape rather than adding to one, and a count of moved files
 * alone never shows it. These are not `FileCount` and must never be added to
 * one.
 */
export type DirCount = number & { readonly kind?: "dirs" };
export const made = (o: Outcome): DirCount => o.created as DirCount;
export const emptied = (o: Outcome): DirCount => o.removed as DirCount;

/**
 * Directories counted by the window from the two trees.
 *
 * `PreviewView` carries no `created` or `removed`, though `Outcome` does. The
 * before and after trees both mark their directories, so the window works the
 * two numbers out by comparing them rather than asking for a field the seam
 * does not have. Property 5 says these matter as much as files moved, and a
 * preview that omitted them would be the one screen that dropped them.
 */
export const dirsCounted = (n: number): DirCount => n as DirCount;

/** `n file`/`n files`, without the parenthesised plural the CLI used to print. */
export const files = (n: FileCount): string => `${n} ${n === 1 ? "file" : "files"}`;
