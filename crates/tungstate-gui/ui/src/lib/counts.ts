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

import type { Outcome, PreviewView, PutBackDone, TidyDone } from "../engine/types";

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
export const made = (o: Outcome | PreviewView): DirCount =>
  ("created" in o ? o.created : 0) as DirCount;
export const emptied = (o: Outcome): DirCount => o.removed as DirCount;

/** `n file`/`n files`, without the parenthesised plural the CLI used to print. */
export const files = (n: FileCount): string => `${n} ${n === 1 ? "file" : "files"}`;
