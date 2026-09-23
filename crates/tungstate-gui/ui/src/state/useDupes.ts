// A duplicate scan, what it found, and what the person decided about it.
//
// Module-level refs, like the other state modules: importing twice gives the
// same scan. The listener is attached once, before anything that can throw,
// for the reason `useTransfer` has in a comment.

import { computed, ref, shallowRef } from "vue";
import { dupes, folders } from "../engine/commands";
import { dupeEvents, type UnlistenFn } from "../engine/events";
import type { Cleared, DupeChoice, Found, ScanProgress } from "../engine/types";

export type Phase = "start" | "scanning" | "found" | "clearing" | "done";

const phase = ref<Phase>("start");
const root = ref<string | null>(null);
const found = shallowRef<Found | null>(null);
const progress = shallowRef<ScanProgress | null>(null);
const cleared = shallowRef<Cleared | null>(null);
const problem = ref<string | null>(null);
const stopping = ref(false);
/** What happens to extra copies, once the person has said. */
const action = ref<string | null>(null);

/** Which groups are ticked, by id. Everything is ticked when a scan lands,
 *  because that is what somebody clearing space came to do; untick what you
 *  want to keep. */
const picked = ref<Set<string>>(new Set());
/** Which copy to keep, where it is not the engine's own choice. */
const keeping = ref<Record<string, string>>({});

let attached = false;
const unlisten: UnlistenFn[] = [];

/** Listen for progress. Called once, from the frame, before any await. */
export async function attachDupeStream() {
  if (attached) return;
  attached = true;
  try {
    unlisten.push(
      await dupeEvents.progress((seen) => {
        progress.value = seen;
      }),
    );
  } catch {
    // A scan that cannot be watched still works; it just says less.
  }
}

export function detachDupeStream() {
  for (const off of unlisten.splice(0)) off();
  attached = false;
}

async function look(target: string) {
  root.value = target;
  found.value = null;
  cleared.value = null;
  problem.value = null;
  progress.value = null;
  stopping.value = false;
  phase.value = "scanning";
  try {
    const answer = await dupes.find(target);
    found.value = answer;
    picked.value = new Set([
      ...answer.groups.map((group) => group.id),
      ...answer.folders.map((group) => group.id),
    ]);
    keeping.value = {};
    phase.value = "found";
  } catch (e) {
    // "stopped" is an answer, not a failure: it is what the Stop button does.
    const said = String(e);
    problem.value = said.includes("stopped") ? null : said;
    phase.value = "start";
  }
}

async function stop() {
  stopping.value = true;
  await dupes.stop().catch(() => {});
}

/** Tick or untick one group. */
function pick(id: string, on: boolean) {
  const next = new Set(picked.value);
  if (on) next.add(id);
  else next.delete(id);
  picked.value = next;
}

/** Keep this copy in this group, instead of the one the engine chose. */
function keep(group: string, path: string) {
  keeping.value = { ...keeping.value, [group]: path };
}

/** The copy a group would keep, as things stand. */
function kept(group: { id: string; keep: string }): string {
  return keeping.value[group.id] ?? group.keep;
}

/** Keep the newest copy in every ticked group, or the oldest.
 *
 *  Files only: a folder has no single date, and inventing one would be a rule
 *  nobody asked for. */
function keepBy(which: "newest" | "oldest") {
  const answer = found.value;
  if (!answer) return;
  const next = { ...keeping.value };
  for (const group of answer.groups) {
    if (!picked.value.has(group.id)) continue;
    const copies = [group.kept, ...group.extras];
    const dated = copies.filter((copy) => copy.mtime !== null);
    if (dated.length < 2) continue;
    dated.sort((a, b) => String(a.mtime).localeCompare(String(b.mtime)));
    next[group.id] = which === "newest" ? dated[dated.length - 1].path : dated[0].path;
  }
  keeping.value = next;
}

/** Files that would be dealt with, given what is ticked and what is kept.
 *
 *  A count of files, never of operations: a plan also removes the directories
 *  it empties, and slice 7b shipped "moved 9 file(s)" for five moved files. */
const chosenFiles = computed(() => {
  const answer = found.value;
  if (!answer) return 0;
  const files = answer.groups
    .filter((group) => picked.value.has(group.id))
    .reduce((count, group) => count + group.extras.length, 0);
  const folders = answer.folders
    .filter((group) => picked.value.has(group.id))
    .reduce((count, group) => count + group.files * group.extras.length, 0);
  return files + folders;
});

const chosenBytes = computed(() => {
  const answer = found.value;
  if (!answer) return 0;
  const files = answer.groups
    .filter((group) => picked.value.has(group.id))
    .reduce((sum, group) => sum + group.size * group.extras.length, 0);
  const folders = answer.folders
    .filter((group) => picked.value.has(group.id))
    .reduce((sum, group) => sum + group.bytes * group.extras.length, 0);
  return files + folders;
});

/** Whether anything ticked was matched on samples rather than read in full. */
const anyUnsure = computed(() => {
  const answer = found.value;
  if (!answer) return false;
  return [...answer.groups, ...answer.folders].some(
    (group) => picked.value.has(group.id) && !group.sure,
  );
});

async function loadAction() {
  try {
    action.value = await dupes.action();
  } catch {
    action.value = null;
  }
}

async function chooseAction(chosen: string) {
  action.value = chosen;
  await dupes.rememberAction(chosen).catch(() => {});
}

async function clear(extras: string) {
  const target = root.value;
  if (!target) return;
  phase.value = "clearing";
  problem.value = null;
  const choices: DupeChoice[] = Object.entries(keeping.value).map(([group, path]) => ({
    group,
    keep: path,
  }));
  try {
    cleared.value = await dupes.clear(target, [...picked.value], choices, extras);
    phase.value = "done";
  } catch (e) {
    problem.value = String(e);
    phase.value = "found";
  }
}

/** Put back what the last clearing set aside.
 *
 *  The same command the folder half uses, given this plan's id: `put_back`
 *  takes any plan, and this is the one place the window knows one outright. */
async function putBack() {
  const target = root.value;
  const done = cleared.value;
  if (!target || !done || !done.reversible) return;
  problem.value = null;
  try {
    const back = await folders.putBack(target, done.plan);
    cleared.value = null;
    phase.value = "start";
    putBackCount.value = back.files;
  } catch (e) {
    problem.value = String(e);
  }
}

/** How many files the last Put it back moved, for the sentence that says so. */
const putBackCount = ref<number | null>(null);

export function useDupes() {
  return {
    phase,
    root,
    found,
    progress,
    cleared,
    problem,
    stopping,
    picked,
    keeping,
    action,
    chosenFiles,
    chosenBytes,
    anyUnsure,
    look,
    stop,
    pick,
    keep,
    kept,
    keepBy,
    clear,
    loadAction,
    chooseAction,
    putBack,
    putBackCount,
    again: () => {
      phase.value = "start";
      cleared.value = null;
      found.value = null;
    },
  };
}
