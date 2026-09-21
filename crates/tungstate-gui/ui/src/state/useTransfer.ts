// One run at a time, and everything the window knows about it.
//
// Must be a singleton: the listeners attach once, and a second copy of this
// module would mean a second set of them and two half-filled ledgers.

import { computed, ref, shallowRef } from "vue";
import { transfers } from "../engine/commands";
import { transferEvents, type UnlistenFn } from "../engine/events";
import type {
  Accepted, Began, ConflictAsk, IdenticalAsk, InterruptedRun, Summary,
} from "../engine/types";

/** One file in the ledger. `state` is an engine string; never bind it to a
 *  class directly -- `lib/tone.ts` maps it, so a new outcome gets a known
 *  look rather than no rule at all. */
export interface Row {
  path: string;
  size: number;
  state: string;
  detail: string | null;
  done: number;
  checked: number;
}

const rows = shallowRef<Row[]>([]);
const live = ref<string | null>(null);
const shape = shallowRef<Began | null>(null);
const atOnce = ref<number | null>(null);
const queued = ref(0);
const summary = shallowRef<Summary | null>(null);
const conflict = shallowRef<ConflictAsk | null>(null);
const identical = shallowRef<IdenticalAsk | null>(null);
const stranded = shallowRef<InterruptedRun[]>([]);
const stopping = ref(false);
const halting = ref(false);
const problem = ref<string | null>(null);
/** Set when the engine cannot reach this window at all. Distinct from a run
 *  that failed: transfers still work, they just report nowhere. */
const deaf = ref<string | null>(null);
const cleaned = ref<string | null>(null);

let attached = false;
const unlisten: UnlistenFn[] = [];

const touch = () => {
  rows.value = [...rows.value];
};
const find = (path: string) => rows.value.find((r) => r.path === path);

/**
 * Attach the listeners. Call once, first, before anything that can throw.
 *
 * They used to be registered last, after four awaits, and when `listen` itself
 * was being refused the whole of `onMounted` stopped there: browsing worked,
 * transfers ran, and the ledger stayed empty for ever with nothing on screen
 * to say why. Hence `deaf`, which is a state this window can draw.
 */
export async function attachTransferStream() {
  if (attached) return;
  attached = true;
  try {
    unlisten.push(
      await transferEvents.began((e) => {
        shape.value = e;
        atOnce.value = e.at_once;
      }),
      await transferEvents.atOnce((n) => (atOnce.value = n)),
      // The whole plan arrives before the first byte, so the ledger shows what
      // is waiting rather than growing a row at a time as things happen.
      await transferEvents.planned((files) => {
        rows.value = files.map((f) => ({
          path: f.path, size: f.size, state: "waiting", detail: null, done: 0, checked: 0,
        }));
      }),
      await transferEvents.started((e) => {
        live.value = e.path;
        const row = find(e.path);
        if (row) {
          row.state = "live";
          row.done = 0;
          row.checked = 0;
        } else {
          // A file the plan did not mention, which happens when a conflict
          // lands it under another name. Better shown than dropped.
          rows.value.push({ path: e.path, size: e.size, state: "live", detail: null, done: 0, checked: 0 });
        }
        touch();
      }),
      await transferEvents.advanced((e) => {
        const row = find(e.path);
        if (!row) return;
        // Back to copying. A file that was being compared and is now sending
        // bytes has to lose the checking state or the row keeps the wrong word.
        row.state = "live";
        row.checked = 0;
        row.done = e.done;
        if (e.total) row.size = e.total;
        touch();
      }),
      await transferEvents.checking((e) => {
        const row = find(e.path);
        if (!row) return;
        row.state = "checking";
        row.done = e.done;
        row.checked = e.total;
        touch();
      }),
      await transferEvents.finished((e) => {
        const row = rows.value.find(
          (r) => r.path === e.path && (r.state === "live" || r.state === "checking"),
        );
        if (row) {
          row.state = e.outcome;
          row.detail = e.detail;
          row.done = row.size;
        }
        if (live.value === e.path) live.value = null;
        touch();
      }),
      await transferEvents.conflict((e) => (conflict.value = e)),
      await transferEvents.identical((e) => (identical.value = e)),
      await transferEvents.queued((e: Accepted) => {
        if (!e.started) queued.value = e.waiting;
      }),
      await transferEvents.done((e) => {
        summary.value = e;
        live.value = null;
        stopping.value = false;
        halting.value = false;
        queued.value = 0;
        atOnce.value = null;
        identical.value = null;
        void refreshStranded();
      }),
      await transferEvents.error((message) => {
        problem.value = message;
        stopping.value = false;
        halting.value = false;
      }),
    );
  } catch (e) {
    deaf.value =
      "This window cannot receive progress from the engine, so transfers will " +
      `run without showing here: ${String(e)}`;
  }
}

export function detachTransferStream() {
  for (const off of unlisten.splice(0)) off();
  attached = false;
}

async function refreshStranded() {
  try {
    stranded.value = await transfers.interrupted();
  } catch {
    // Shown by whatever asked for it; a failure here must not blank the run.
  }
}

function clearRun() {
  rows.value = [];
  summary.value = null;
  problem.value = null;
  cleaned.value = null;
  shape.value = null;
  atOnce.value = null;
}

async function resume(link: string) {
  clearRun();
  try {
    await transfers.resume(link);
  } catch (e) {
    problem.value = String(e);
  }
  await refreshStranded();
}

async function discard(link: string) {
  try {
    const freed = await transfers.discard(link);
    problem.value = null;
    cleaned.value = String(freed);
  } catch (e) {
    problem.value = String(e);
  }
  await refreshStranded();
}

/** Files settled, out of files planned. Both counts of files, and neither is
 *  a count of operations. */
const settled = computed(() => rows.value.filter((r) => r.state !== "waiting" && r.state !== "live" && r.state !== "checking").length);
const running = computed(() => live.value !== null || rows.value.some((r) => r.state === "live" || r.state === "checking"));

export function useTransfer() {
  return {
    rows, live, shape, atOnce, queued, summary, conflict, identical,
    stranded, stopping, halting, problem, deaf, cleaned,
    settled, running,
    refreshStranded, clearRun, resume, discard,
    async answerConflict(action: string, applyToAll: boolean) {
      conflict.value = null;
      await transfers.resolveConflict(action, applyToAll);
    },
    async answerIdentical(remove: boolean, applyToAll: boolean) {
      identical.value = null;
      await transfers.resolveIdentical(remove, applyToAll);
    },
    async cancel() {
      stopping.value = true;
      await transfers.cancel();
    },
    async stopNow() {
      halting.value = true;
      await transfers.stopNow();
    },
    async setAtOnce(files: number) {
      await transfers.setAtOnce(files);
    },
  };
}
