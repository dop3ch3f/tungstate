// A duplicate scan, what it found, and what the person decided about it.
//
// Module-level refs, like the other state modules: importing twice gives the
// same scan. The listener is attached once, before anything that can throw,
// for the reason `useTransfer` has in a comment.

import { computed, ref, shallowRef } from "vue";
import { dupes } from "../engine/commands";
import { dupeEvents, type UnlistenFn } from "../engine/events";
import type { Claim, Cleared, DupeKind, DupeRow, Found, ScanProgress } from "../engine/types";
import { useTable } from "../lib/table";

export type Phase = "start" | "scanning" | "found" | "clearing" | "done";

/** Which drawer of the left pane is open. */
export interface Drawer {
  claim: Claim;
  kind: DupeKind | null;
}

const phase = ref<Phase>("start");
const root = ref<string | null>(null);
const found = shallowRef<Found | null>(null);
const progress = shallowRef<ScanProgress | null>(null);
const cleared = shallowRef<Cleared | null>(null);
const problem = ref<string | null>(null);
const stopping = ref(false);
/** What happens to extra copies, once the person has said. */
const action = ref<string | null>(null);
/** Places scanned before, so a second look is one click. */
const recent = shallowRef<string[]>([]);
/** Whether to look for files that merely resemble each other, which costs a
 *  decode of every picture and video and is therefore asked for rather than
 *  assumed. */
const alsoSimilar = ref(true);

/** Which copies are ticked, by path.
 *
 *  Copies rather than groups: every checkbox on screen is its own copy, and
 *  keeping two of four is a thing people want. */
const ticked = ref<Set<string>>(new Set());
/** Which drawer is open on the left. */
const drawer = ref<Drawer>({ claim: "identical", kind: null });
/** Which copy the preview is showing. */
const highlighted = ref<string | null>(null);
/** Which groups are open in the middle pane. */
const opened = ref<Set<string>>(new Set());

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

/** Everything a scan ticks for you: the extra copies of files that are
 *  provably identical, and nothing that was matched on a resemblance.
 *
 *  The asymmetry is the whole point. An identical group has proof, so the work
 *  is already done and unticking is the exception. A resemblance is a guess,
 *  and a guess that arrives pre-agreed is how somebody loses a photograph. */
function ticksFor(answer: Found): Set<string> {
  const next = new Set<string>();
  for (const row of answer.rows) {
    if (row.claim !== "identical") continue;
    for (const copy of row.copies) {
      if (!copy.keep) next.add(copy.path);
    }
  }
  return next;
}

async function look(target: string) {
  root.value = target;
  // The last run's result is not this run's: leaving it on screen would let
  // "Put 2 files back" sit above a fresh scan that put nothing anywhere.
  putBackCount.value = null;
  found.value = null;
  table.clear();
  cleared.value = null;
  problem.value = null;
  progress.value = null;
  stopping.value = false;
  highlighted.value = null;
  opened.value = new Set();
  phase.value = "scanning";
  try {
    const answer = await dupes.find(target, alsoSimilar.value);
    found.value = answer;
    ticked.value = ticksFor(answer);
    drawer.value = { claim: firstClaimWithRows(answer), kind: null };
    // The first group opens, so the screen arrives showing what a group is
    // rather than a list of closed drawers.
    const first = answer.rows.find((row) => row.claim === drawer.value.claim);
    opened.value = new Set(first ? [first.id] : []);
    highlighted.value = first?.copies[0]?.path ?? null;
    phase.value = "found";
    void loadRecent();
  } catch (e) {
    // "stopped" is an answer, not a failure: it is what the Stop button does.
    const said = String(e);
    problem.value = said.includes("stopped") ? null : said;
    phase.value = "start";
  }
}

function firstClaimWithRows(answer: Found): Claim {
  for (const claim of ["identical", "same", "similar"] as Claim[]) {
    if (answer.rows.some((row) => row.claim === claim)) return claim;
  }
  return "identical";
}

async function stop() {
  stopping.value = true;
  await dupes.stop().catch(() => {});
}

/** Tick or untick one copy. */
function tick(path: string, on: boolean) {
  const next = new Set(ticked.value);
  if (on) next.add(path);
  else next.delete(path);
  ticked.value = next;
}

/** Open or close one group in the middle pane. */
function open(id: string, on: boolean) {
  const next = new Set(opened.value);
  if (on) next.add(id);
  else next.delete(id);
  opened.value = next;
}

/** The rows in the open drawer, before any search. */
const inDrawer = computed<DupeRow[]>(() => {
  const answer = found.value;
  if (!answer) return [];
  return answer.rows.filter(
    (row) => row.claim === drawer.value.claim && (!drawer.value.kind || row.kind === drawer.value.kind),
  );
});

/** Search and sort over the open drawer. Every rule below acts on `shown`, so
 *  a rule only ever ticks copies somebody can see: filtering to "IMG" and
 *  pressing "Tick every extra" must not tick a group the filter is hiding. */
const table = useTable(inDrawer, {
  columns: [
    { key: "reclaim", value: (row) => row.reclaimable },
    { key: "name", value: (row) => row.copies[0]?.name ?? "" },
    { key: "copies", value: (row) => row.copies.length },
  ],
  text: (row) => row.copies.map((copy) => copy.path).join(" "),
  sort: { key: "reclaim", dir: -1 },
});
const shown = table.shown;

/** How many copies of a row are ticked. */
function tickedIn(row: DupeRow): number {
  return row.copies.filter((copy) => ticked.value.has(copy.path)).length;
}

/** The copy the preview is showing, or the first one worth showing. */
const showing = computed(() => {
  const rows = shown.value;
  const want = highlighted.value;
  for (const row of rows) {
    for (const copy of row.copies) {
      if (copy.path === want) return { row, copy };
    }
  }
  const first = rows[0];
  return first ? { row: first, copy: first.copies[0] } : null;
});

/** Set the ticks in the open drawer by a rule.
 *
 *  A rule never acts. It moves the checkboxes and leaves the list on screen,
 *  so the preview still gets the last word. Every one of them keeps a copy by
 *  construction, and the engine checks again anyway. */
function keepBy(which: "newest" | "oldest" | "biggest" | "none" | "all") {
  const next = new Set(ticked.value);
  for (const row of shown.value) {
    for (const copy of row.copies) next.delete(copy.path);
    if (which === "none") continue;
    if (which === "all") {
      // Everything but the copy the pass chose, which is the one rule that
      // cannot empty a group.
      for (const copy of row.copies) if (!copy.keep) next.add(copy.path);
      continue;
    }
    const keeping = pick(row, which);
    if (!keeping) continue;
    for (const copy of row.copies) if (copy.path !== keeping) next.add(copy.path);
  }
  ticked.value = next;
}

/** Which copy a rule would keep, or null when the row cannot answer. */
function pick(row: DupeRow, which: "newest" | "oldest" | "biggest"): string | null {
  if (which === "biggest") {
    let best = row.copies[0];
    for (const copy of row.copies) if (copy.size > best.size) best = copy;
    return best?.path ?? null;
  }
  // Dates only, and only where there are two of them. A folder has no single
  // date, and inventing one would be a rule nobody asked for.
  if (row.folder) return null;
  const dated = row.copies.filter((copy) => copy.mtime !== null);
  if (dated.length < 2) return null;
  dated.sort((a, b) => String(a.mtime).localeCompare(String(b.mtime)));
  return which === "newest" ? dated[dated.length - 1].path : dated[0].path;
}

/** Keep whichever copy sits under this directory, in the open drawer. */
function keepIn(directory: string) {
  const inside = (path: string) => path === directory || path.startsWith(`${directory}/`);
  const next = new Set(ticked.value);
  for (const row of shown.value) {
    const keeping = row.copies.find((copy) => inside(copy.path));
    if (!keeping) continue;
    for (const copy of row.copies) {
      if (copy.path === keeping.path) next.delete(copy.path);
      else next.add(copy.path);
    }
  }
  ticked.value = next;
}

/** Files that would be dealt with, given what is ticked.
 *
 *  A count of files, never of operations: a plan also removes the directories
 *  it empties, and slice 7b shipped "moved 9 file(s)" for five moved files. */
const chosenFiles = computed(() => {
  const answer = found.value;
  if (!answer) return 0;
  let total = 0;
  for (const row of answer.rows) {
    const count = tickedIn(row);
    total += row.folder ? count * row.files : count;
  }
  return total;
});

const chosenBytes = computed(() => {
  const answer = found.value;
  if (!answer) return 0;
  let total = 0;
  for (const row of answer.rows) {
    for (const copy of row.copies) if (ticked.value.has(copy.path)) total += copy.size;
  }
  return total;
});

/** Whether anything ticked was matched on samples rather than read in full. */
const anyUnsure = computed(() => {
  const answer = found.value;
  if (!answer) return false;
  return answer.rows.some((row) => !row.sure && tickedIn(row) > 0);
});

/** Whether any ticked copy came from a resemblance rather than from proof.
 *
 *  The confirm sheet says so out loud, because a guess and an irreversible
 *  delete is the one combination that can lose somebody a photograph. */
const anyGuessed = computed(() => {
  const answer = found.value;
  if (!answer) return false;
  return answer.rows.some((row) => row.claim !== "identical" && tickedIn(row) > 0);
});

/** How much each drawer holds, for the left pane. */
const tallies = computed(() => {
  const answer = found.value;
  const out: Record<string, { rows: number; bytes: number }> = {};
  if (!answer) return out;
  for (const row of answer.rows) {
    for (const key of [row.claim, `${row.claim}:${row.kind}`]) {
      const at = (out[key] ??= { rows: 0, bytes: 0 });
      at.rows += 1;
      at.bytes += row.reclaimable;
    }
  }
  return out;
});

async function loadRecent() {
  try {
    recent.value = await dupes.recent();
  } catch {
    recent.value = [];
  }
}

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
  putBackCount.value = null;
  try {
    cleared.value = await dupes.clear(target, [...ticked.value], alsoSimilar.value, extras);
    phase.value = "done";
  } catch (e) {
    problem.value = String(e);
    phase.value = "found";
  }
}

/** Put back what the last clearing set aside.
 *
 *  The duplicates window's own undo rather than the folder half's, which walks
 *  up to a policy file and refuses without one. This window scans anything,
 *  governed or not, so an undo that needs rules is an undo that is missing
 *  exactly when somebody wants it. */
async function putBack() {
  const target = root.value;
  const done = cleared.value;
  if (!target || !done || !done.reversible) return;
  problem.value = null;
  try {
    const files = await dupes.putBack(target, done.plan);
    cleared.value = null;
    phase.value = "start";
    putBackCount.value = files;
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
    ticked,
    drawer,
    opened,
    highlighted,
    shown,
    inDrawer,
    table,
    showing,
    tallies,
    action,
    alsoSimilar,
    chosenFiles,
    chosenBytes,
    anyUnsure,
    anyGuessed,
    look,
    stop,
    tick,
    tickedIn,
    open,
    keepBy,
    keepIn,
    clear,
    recent,
    loadRecent,
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
