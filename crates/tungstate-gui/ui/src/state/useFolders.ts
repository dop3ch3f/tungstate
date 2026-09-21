// The folder half's state, in one place.
//
// Module-level refs, exported through a function. Importing this twice gives
// the same refs, which is a store in a dozen lines with no dependency and no
// ceremony. Two things follow from that and are worth knowing: it never
// resets, which is right for a single-window desktop application and would be
// wrong the moment there were two windows; and anything that wants to react to
// it must import this module rather than receive it as a prop.

import { computed, ref, shallowRef } from "vue";
import { folders } from "../engine/commands";
import type { FolderView, Learned, Outcome, PreviewView, TidyDone } from "../engine/types";

/** Where the person is in the one flow this half has. */
export type Phase = "start" | "reading" | "choosing" | "previewing";

const phase = ref<Phase>("start");
const root = ref<string | null>(null);
const registered = shallowRef<FolderView[]>([]);
const learned = shallowRef<Learned | null>(null);
const outcomes = shallowRef<Outcome[]>([]);
const preview = shallowRef<PreviewView | null>(null);
const tidied = shallowRef<TidyDone | null>(null);
const putBackCount = ref<number | null>(null);
const busy = ref<string | null>(null);
const problem = ref<string | null>(null);

/** Run an engine call, keeping the one error slot and the one busy slot
 *  honest. Errors cross as bare sentences, so there is nothing to unwrap. */
async function run<T>(what: string, work: () => Promise<T>): Promise<T | null> {
  busy.value = what;
  problem.value = null;
  try {
    return await work();
  } catch (e) {
    problem.value = String(e);
    return null;
  } finally {
    busy.value = null;
  }
}

async function listRegistered() {
  const list = await run("Reading your folders", () => folders.governed());
  if (list) registered.value = list;
}

/** Look at a folder without registering it.
 *
 *  `learn_folder` and `compare_folder` both take a bare path, which is what
 *  lets this be the first thing the app does. Nothing is written, nothing is
 *  governed, and the person has not had to learn the word "layout" yet. */
async function look(path: string) {
  root.value = path;
  learned.value = null;
  outcomes.value = [];
  preview.value = null;
  tidied.value = null;
  putBackCount.value = null;
  phase.value = "reading";

  // One pass each, together: both walk the folder, so serialising them would
  // double the wait on the very first screen anyone sees.
  const got = await run("Reading the folder", async () =>
    Promise.all([folders.learn(path), folders.compare(path)]),
  );
  if (!got) {
    phase.value = "start";
    return;
  }
  [learned.value, outcomes.value] = got;
  phase.value = "choosing";
}

/** Open a folder that is already governed, straight to its preview. */
async function open(path: string) {
  root.value = path;
  tidied.value = null;
  putBackCount.value = null;
  await refresh();
  if (preview.value) phase.value = "previewing";
}

async function refresh() {
  if (!root.value) return;
  const view = await run("Working out what would move", () => folders.preview(root.value!));
  if (view) preview.value = view;
}

/** Register the folder, write the chosen rules, then show what they would do.
 *
 *  Three calls and not one, because the seam has three. Registering is
 *  reversible and writes nothing into the folder; writing rules puts a file
 *  in it and moves no files; only the preview screen has a button that does. */
async function choose(layout: string) {
  const path = root.value;
  if (!path) return;
  const ok = await run("Giving the folder its rules", async () => {
    await folders.govern(path);
    await folders.giveRules(path, layout);
    return true;
  });
  if (!ok) return;
  await listRegistered();
  await refresh();
  if (preview.value) phase.value = "previewing";
}

async function tidy() {
  const path = root.value;
  if (!path) return;
  // No events exist for this, so it is one blocking call and the screen says
  // so. See `docs/SEAM.md`, "what is deliberately not in the seam yet".
  const done = await run("Tidying. This does not report progress", () => folders.tidy(path));
  if (!done) return;
  tidied.value = done;
  putBackCount.value = null;
  await refresh();
}

async function putBack(plan: number) {
  const path = root.value;
  if (!path) return;
  const done = await run("Putting it back", () => folders.putBack(path, plan));
  if (!done) return;
  putBackCount.value = done.files;
  tidied.value = null;
  await refresh();
}

async function forget(path: string) {
  await run("Forgetting the folder", () => folders.forget(path));
  await listRegistered();
  if (root.value === path) {
    root.value = null;
    phase.value = "start";
  }
}

async function pick() {
  const chosen = await run("Waiting for you to choose a folder", () => folders.pick());
  if (chosen) await look(chosen);
}

/** The folder in hand, as the registry knows it. Null before it is governed. */
const current = computed(() =>
  registered.value.find((f) => f.root === root.value) ?? null,
);

/** Whether `give_rules` would refuse. It will not overwrite a policy, so a
 *  folder that already has one can only be compared against, not re-filed. */
const hasRulesAlready = computed(() => current.value?.has_rules ?? false);

export function useFolders() {
  return {
    phase,
    root,
    registered,
    learned,
    outcomes,
    preview,
    tidied,
    putBackCount,
    busy,
    problem,
    current,
    hasRulesAlready,
    listRegistered,
    look,
    open,
    refresh,
    choose,
    tidy,
    putBack,
    forget,
    pick,
    back: () => {
      phase.value = "start";
      problem.value = null;
    },
  };
}
