// The clipboard: files chosen for Copy or Move, carried while you browse to
// where they go, then "Copy here" or "Move here". As ES File Explorer does it.
//
// A Move is the engine's safe move: each file is sent, read back and
// checked, and only then is the original removed (`source_policy: delete`).
// Free up's findings ride the same bar, with their own way to land.

import { computed, shallowRef } from "vue";
import { transfers } from "../../engine/commands";
import { legsFor } from "../lib/paths";

export type Op = "copy" | "move";

export interface Clip {
  op: Op;
  /** What the bar says it holds: "3 photos". */
  what: string;
  bytes: number;
  /** Where they came from, as the bar words it. */
  from: string;
  /** Locations; empty for Free up, which names its photos itself. */
  paths: string[];
  /** Free up's own landing, given the folder chosen. */
  land?: (destination: string) => Promise<void>;
}

/** What a phone transfer asks of the engine, besides the files. */
const PHONE = {
  verify: "readback",
  // Two files with one name: keep both, never ask mid-run on a phone.
  on_conflict: "rename",
  save_as: null,
} as const;

const clip = shallowRef<Clip | null>(null);
/** The transfer a paste started, for the Moving screen to follow. */
const started = shallowRef<Started | null>(null);

/** A paste that became a transfer: enough to follow it, and to send the
 *  rest again if it is paused part-way. */
export interface Started {
  job: number;
  op: Op;
  what: string;
  /** The folder's name, for the screen. */
  to: string;
  destination: string;
  paths: string[];
  /** Picked up again from Files home after it stopped part-way. */
  resumed?: boolean;
}

function hold(next: Clip) {
  clip.value = next;
}

function drop() {
  clip.value = null;
}

/** The request a paste sends, apart from the clipboard, so it can be tested. */
export function requestFor(op: Op, paths: string[], destination: string) {
  return { legs: legsFor(paths, destination), source_policy: op === "move" ? "delete" : "keep", ...PHONE };
}

/** Land what is held in `destination`. Throws with the engine's sentence. */
async function paste(destination: string, label: string) {
  const held = clip.value;
  if (!held) return;
  if (held.land) {
    await held.land(destination);
  } else {
    const accepted = await transfers.start(requestFor(held.op, held.paths, destination));
    started.value = { job: accepted.job, op: held.op, what: held.what, to: label, destination, paths: held.paths };
  }
  clip.value = null;
}

/** Send again what a paused transfer had not finished. Moved files are gone
 *  from their folders, so a folder sent again carries only what is left. */
async function again(paths: string[]) {
  const was = started.value;
  if (!was || !paths.length) return;
  const accepted = await transfers.start(requestFor(was.op, paths, was.destination));
  started.value = { ...was, job: accepted.job, paths };
}

/** Follow a transfer that did not start from the clipboard: a stopped one
 *  picked up again from Files home. */
function follow(run: Started) {
  started.value = run;
}

export function useClipboard() {
  return { clip, started, held: computed(() => clip.value !== null), hold, drop, paste, again, follow, done: () => (started.value = null) };
}
