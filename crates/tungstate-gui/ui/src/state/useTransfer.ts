// The transfer queue, and everything the window knows about each transfer in it.
//
// One runs at a time and the rest wait, like downloads. The engine announces
// each transfer as it starts (`transfer://job`) and every file event after
// that is that transfer's until its `done` or `error`, so each keeps its own
// ledger and none is ever written into another's.
//
// Must be a singleton: the listeners attach once, and a second copy of this
// module would mean a second set of them and two half-filled ledgers.

import { computed, ref, shallowRef } from "vue";
import { transfers } from "../engine/commands";
import { transferEvents, type UnlistenFn } from "../engine/events";
import { ordinal } from "../lib/format";
import { useBusy } from "./useBusy";
import type {
  Accepted, Began, ConflictAsk, IdenticalAsk, InterruptedRun, JobView, QueueView, Summary,
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

/** A transfer that has ended, with its own ledger. `summary` is null when it
 *  could not carry on, and `problem` says why. */
export interface Ran {
  job: JobView;
  rows: Row[];
  summary: Summary | null;
  problem: string | null;
  at: number;
}

/** How many ended transfers are kept on screen. Older ones are in History. */
const KEEP = 8;

const queue = shallowRef<QueueView>({ running: null, waiting: [] });
/** The transfer the file events below belong to. */
const current = shallowRef<JobView | null>(null);
const rows = shallowRef<Row[]>([]);
const live = ref<string | null>(null);
const shape = shallowRef<Began | null>(null);
const atOnce = ref<number | null>(null);
const finished = shallowRef<Ran[]>([]);
/** "Added to the queue, 2nd in line", for the transfer just queued. */
const placed = ref<{ job: number; line: string } | null>(null);
const conflict = shallowRef<ConflictAsk | null>(null);
const identical = shallowRef<IdenticalAsk | null>(null);
const stranded = shallowRef<InterruptedRun[]>([]);
const stopping = ref(false);
const halting = ref(false);
/** A command that failed. A transfer that fails says so in its own row. */
const problem = ref<string | null>(null);
/** Which tab Transfer opens on next, for a caller elsewhere that started
 *  something there to watch. Read and cleared as Transfer opens. */
const openOn = ref<"runs" | "links" | null>(null);
/** Set when the engine cannot reach this window at all. Distinct from a run
 *  that failed: transfers still work, they just report nowhere. */
const deaf = ref<string | null>(null);
const cleaned = ref<string | null>(null);

let attached = false;
const unlisten: UnlistenFn[] = [];

/** The running ledger's rows by path. A drain of 10,000 photographs sends
 *  three events a file, and scanning the list for each froze the window. */
const byPath = new Map<string, Row[]>();
const add = (row: Row) => {
  const same = byPath.get(row.path);
  if (same) same.push(row);
  else byPath.set(row.path, [row]);
};

// Rows change in place and are republished at most once a frame: once per
// event redrew the whole ledger thousands of times a second.
let republishing = false;
const touch = () => {
  if (republishing) return;
  republishing = true;
  requestAnimationFrame(() => {
    republishing = false;
    rows.value = [...rows.value];
  });
};
/** The row a file event is about. An exchange can list one name twice, once
 *  per direction, so the row in the expected state is preferred. */
const find = (path: string, ...states: string[]) => {
  const same = byPath.get(path);
  return same?.find((r) => states.includes(r.state)) ?? same?.[0];
};

function beginLedger(job: JobView | null) {
  current.value = job;
  byPath.clear();
  rows.value = [];
  live.value = null;
  shape.value = null;
  atOnce.value = null;
}

/** A transfer ended: its ledger moves to Recently finished. */
function settle(id: number, summary: Summary | null, why: string | null) {
  const job = current.value?.id === id
    ? current.value
    : { id, name: null, source: "", destination: "", exchange: false, removes_originals: false, files: null, bytes: null };
  finished.value = [{ job, rows: rows.value, summary, problem: why, at: Date.now() }, ...finished.value].slice(0, KEEP);
  beginLedger(null);
  identical.value = null;
  void refreshStranded();
}

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
      await transferEvents.queue((q) => {
        queue.value = q;
        // Stop clears the queue, so an empty one is also the end of stopping.
        if (!q.running && !q.waiting.length) {
          stopping.value = false;
          halting.value = false;
        }
      }),
      await transferEvents.job((job) => {
        beginLedger(job);
        if (placed.value?.job === job.id) placed.value = null;
      }),
      await transferEvents.began((e) => {
        shape.value = e;
        atOnce.value = e.at_once;
      }),
      await transferEvents.atOnce((n) => (atOnce.value = n)),
      // The whole plan arrives before the first byte, so the ledger shows what
      // is waiting rather than growing a row at a time. Added to, not
      // replaced: the second leg of an exchange plans after the first ran.
      await transferEvents.planned((files) => {
        const planned = files.map((f) => ({ path: f.path, size: f.size, state: "waiting", detail: null, done: 0, checked: 0 }));
        planned.forEach(add);
        rows.value = [...rows.value, ...planned];
      }),
      await transferEvents.started((e) => {
        live.value = e.path;
        const row = find(e.path, "waiting");
        if (row) {
          row.state = "live";
          row.done = 0;
          row.checked = 0;
        } else {
          // A file the plan did not mention, which happens when a conflict
          // lands it under another name. Better shown than dropped.
          const unplanned = { path: e.path, size: e.size, state: "live", detail: null, done: 0, checked: 0 };
          add(unplanned);
          rows.value.push(unplanned);
        }
        touch();
      }),
      await transferEvents.advanced((e) => {
        const row = find(e.path, "live", "checking");
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
        const row = find(e.path, "live", "checking");
        if (!row) return;
        row.state = "checking";
        row.done = e.done;
        row.checked = e.total;
        touch();
      }),
      await transferEvents.finished((e) => {
        const row = byPath.get(e.path)?.find((r) => r.state === "live" || r.state === "checking");
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
      await transferEvents.done(({ job, ...summary }) => settle(job, summary, null)),
      await transferEvents.error(({ job, message }) => settle(job, null, message)),
    );
    queue.value = await transfers.queue();
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

/** What the engine said about a transfer just asked for. Only a queued one
 *  needs saying: a started one shows up running. */
function accepted(a: Accepted) {
  problem.value = null;
  placed.value = a.started ? null : { job: a.job, line: `Added to the queue, ${ordinal(a.waiting + 1)} in line.` };
}

// Keyed by link, or by queued transfer: the row stays up until the engine
// says otherwise, and a second click in that time would act twice.
const doing = useBusy();

const resume = (link: string) =>
  doing.run(link, async () => {
    try {
      accepted(await transfers.resume(link));
    } catch (e) {
      problem.value = String(e);
    }
    await refreshStranded();
  });

const discard = (link: string) =>
  doing.run(link, async () => {
    try {
      const freed = await transfers.discard(link);
      problem.value = null;
      cleaned.value = String(freed);
    } catch (e) {
      problem.value = String(e);
    }
    await refreshStranded();
  });

const unqueue = (job: number) =>
  doing.run(`queued-${job}`, async () => {
    try {
      // False means it started first; the queue event already shows it running.
      await transfers.removeFromQueue(job);
      if (placed.value?.job === job) placed.value = null;
    } catch (e) {
      problem.value = String(e);
    }
  });

/** Files settled, out of files planned. Both counts of files, and neither is
 *  a count of operations. */
const settled = computed(() => rows.value.filter((r) => r.state !== "waiting" && r.state !== "live" && r.state !== "checking").length);
const running = computed(() => current.value !== null || queue.value.running !== null);

export function useTransfer() {
  return {
    queue, current, rows, live, shape, atOnce, finished, placed, conflict, identical,
    stranded, stopping, halting, problem, deaf, cleaned, openOn,
    /** A stopped run being picked up or cleaned up. */
    settling: (link: string) => doing.busy(link),
    /** A queued transfer being taken out. */
    unqueuing: (job: number) => doing.busy(`queued-${job}`),
    settled, running,
    refreshStranded, accepted, resume, discard, unqueue,
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
      try {
        await transfers.cancel();
      } catch (e) {
        stopping.value = false;
        problem.value = String(e);
      }
    },
    async stopNow() {
      halting.value = true;
      try {
        await transfers.stopNow();
      } catch (e) {
        halting.value = false;
        problem.value = String(e);
      }
    },
    async setAtOnce(files: number) {
      await transfers.setAtOnce(files);
    },
  };
}
