// The Sync section, and everything the window knows about its syncs.
//
// A singleton, like the others: the listeners attach once, and a second copy
// of this module would mean two ledgers half filled from one stream.
//
// A run is always a preview first. What the person says about conflicts goes
// into the next preview, so the numbers on screen are the numbers of the run
// that will happen, and the run is handed that preview's fingerprint: the
// engine refuses a run that has changed since.

import { computed, ref, shallowRef } from "vue";
import { syncs, transfers } from "../engine/commands";
import { syncEvents, type UnlistenFn } from "../engine/events";
import type {
  FollowingView, PastSync, SyncAsk, SyncConflict, SyncPreview, SyncRan, SyncUndone, SyncView,
} from "../engine/types";
import { latest } from "../lib/latest";
import { reason } from "../lib/syncwords";

export type Phase = "list" | "make" | "one" | "reading" | "preview" | "running" | "done";

/** One file in the run's ledger. `leg` is which pair of members it is in. */
export interface SyncRow {
  leg: number;
  path: string;
  size: number;
  state: string;
  detail: string | null;
  done: number;
}

/** What the person chose for one conflict. A member's name, `both`, or
 *  `leave`. Leaving it is the default, and a left conflict is dealt with the
 *  way the sync is set to deal with conflicts. */
export type Choice = string;

const phase = ref<Phase>("list");
const list = shallowRef<SyncView[]>([]);
const past = shallowRef<PastSync[]>([]);
const loaded = ref(false);
const current = shallowRef<SyncView | null>(null);
const preview = shallowRef<SyncPreview | null>(null);
/** Files or folders being forgotten, when this preview is a forget. */
const forgetting = shallowRef<string[]>([]);
const choices = ref<Record<string, Choice>>({});
/** Every conflict shown since this preview began. One that has been answered
 *  drops out of the next preview, because the engine has settled it; it stays
 *  here so the answer can still be seen and changed. */
const seen = shallowRef<SyncConflict[]>([]);
/** The refusals shown were read, and the person said go anyway. */
const confirmed = ref(false);
/** A new preview is being made while the last one stays on screen. */
const rechecking = ref(false);
const rows = shallowRef<SyncRow[]>([]);
const leg = ref<{ index: number; from: string; to: string } | null>(null);
const ran = shallowRef<SyncRan | null>(null);
const undone = shallowRef<SyncUndone | null>(null);
const stopping = ref(false);
const halting = ref(false);
const problem = ref<string | null>(null);
const busy = ref<number | null>(null);
/** Syncs kept in step: each member's state, and which stopped to ask. */
const followingNow = shallowRef<FollowingView>({ members: [], held: [] });
/** Syncs waiting behind the one running, from the launch sheet. */
const waiting = ref(0);

let attached = false;
const unlisten: UnlistenFn[] = [];
const touch = () => (rows.value = [...rows.value]);
const find = (path: string) =>
  rows.value.find((r) => r.path === path && r.leg === (leg.value?.index ?? 0));

export async function attachSyncStream() {
  if (attached) return;
  attached = true;
  try {
    unlisten.push(
      await syncEvents.leg((e) => (leg.value = { index: e.index, from: e.from, to: e.to })),
      // Each leg plans its own files; they join the ledger rather than
      // replacing it, so the whole run reads top to bottom.
      await syncEvents.planned((files) => {
        const index = leg.value?.index ?? 0;
        rows.value = [
          ...rows.value,
          ...files.map((f) => ({ leg: index, path: f.path, size: f.size, state: "waiting", detail: null, done: 0 })),
        ];
      }),
      await syncEvents.started((e) => {
        const row = find(e.path);
        if (row) {
          row.state = "live";
          row.done = 0;
          touch();
        }
      }),
      await syncEvents.advanced((e) => {
        const row = find(e.path);
        if (!row) return;
        row.state = "live";
        row.done = e.done;
        touch();
      }),
      await syncEvents.checking((e) => {
        const row = find(e.path);
        if (!row) return;
        row.state = "checking";
        row.done = e.done;
        touch();
      }),
      await syncEvents.finished((e) => {
        const row = rows.value.find(
          (r) => r.path === e.path && (r.state === "live" || r.state === "checking"),
        );
        if (row) {
          row.state = e.outcome;
          row.detail = e.detail;
          row.done = row.size;
          touch();
        }
      }),
      await syncEvents.done((e) => {
        stopping.value = halting.value = false;
        waiting.value = Math.max(0, waiting.value - 1);
        // A run kept in step happens on its own; it must not pull the screen
        // away from whatever the person is looking at. Only a run they are
        // watching moves on to its result.
        if (current.value?.name === e.sync && phase.value === "running") {
          ran.value = e;
          phase.value = "done";
        }
        // A held sync that a person has now run is kept in step again.
        if (followingNow.value.held.includes(e.sync)) void syncs.resume(e.sync);
        void loadPast();
      }),
      await syncEvents.following((e) => (followingNow.value = e)),
      await syncEvents.error((e) => {
        problem.value =
          e.kind === "changed"
            ? `${e.sync} changed after it was previewed, so nothing ran. Look at the preview again.`
            : `${e.sync}: ${e.message}`;
        stopping.value = halting.value = false;
        waiting.value = Math.max(0, waiting.value - 1);
        if (current.value?.name === e.sync && phase.value === "running") phase.value = "one";
      }),
    );
  } catch (e) {
    problem.value = `This window cannot hear sync progress, so runs will not show here: ${String(e)}`;
  }
  // Syncs kept in step start with the window, before anyone opens Sync.
  await loadFollowing();
}

export function detachSyncStream() {
  for (const off of unlisten.splice(0)) off();
  attached = false;
}

async function load() {
  try {
    list.value = await syncs.list();
    if (current.value) {
      current.value = list.value.find((s) => s.name === current.value?.name) ?? null;
    }
  } catch (e) {
    problem.value = reason(e);
  } finally {
    loaded.value = true;
  }
  await Promise.all([loadPast(), loadFollowing()]);
}

async function loadFollowing() {
  try {
    followingNow.value = await syncs.following();
  } catch {
    // The statuses are a nicety; the syncs themselves still show.
  }
}

async function loadPast() {
  try {
    past.value = await syncs.past();
  } catch (e) {
    problem.value = reason(e);
  }
}

/** The answers as the engine takes them: left conflicts are not sent. */
function ask(): SyncAsk | null {
  const resolve = Object.entries(choices.value)
    .filter(([, choice]) => choice !== "leave")
    .map(([path, choice]) => ({ path, keep: choice === "both" ? null : choice }));
  if (forgetting.value.length) return { forget: forgetting.value, only: true };
  if (!resolve.length) return null;
  return { resolve };
}

const previews = latest();

async function look() {
  if (!current.value) return;
  const name = current.value.name;
  // Answering a conflict previews again; the screen keeps showing the last
  // preview meanwhile rather than blanking to a loader on every click.
  const again = phase.value === "preview";
  if (again) rechecking.value = true;
  else phase.value = "reading";
  problem.value = null;
  confirmed.value = false;
  const ticket = previews.take();
  try {
    const answer = await syncs.preview(name, ask());
    // A quicker second answer to a conflict previews again; only the newest
    // matches what is chosen on screen, and its fingerprint is the one run.
    if (previews.stale(ticket)) return;
    preview.value = answer;
    const known = new Set(seen.value.map((c) => c.path));
    seen.value = [...seen.value, ...answer.conflicts.filter((c) => !known.has(c.path))];
    phase.value = "preview";
  } catch (e) {
    if (previews.stale(ticket)) return;
    problem.value = reason(e);
    if (!again) phase.value = "one";
  } finally {
    if (!previews.stale(ticket)) rechecking.value = false;
  }
}

async function start(name: string, withAsk: SyncAsk | null, fingerprint: string, yes: boolean) {
  const queued = await syncs.run(name, withAsk, fingerprint, yes);
  if (!queued.started) waiting.value = queued.waiting;
}

export function useSync() {
  return {
    phase, list, past, loaded, current, preview, forgetting, choices, seen, confirmed, rechecking,
    rows, leg, ran, undone, stopping, halting, problem, busy, waiting, followingNow,
    /** A member's status, when its sync is kept in step. */
    statusOf: (sync: string, member: string) =>
      followingNow.value.members.find((m) => m.sync === sync && (m.member === member || m.member === "")) ?? null,
    isHeld: (sync: string) => followingNow.value.held.includes(sync),
    load, loadPast, look,
    /** Files settled in the ledger. */
    settled: computed(() => rows.value.filter((r) => !["waiting", "live", "checking"].includes(r.state)).length),
    running: computed(() => phase.value === "running" && !ran.value),
    open(sync: SyncView) {
      current.value = sync;
      preview.value = null;
      forgetting.value = [];
      choices.value = {};
      seen.value = [];
      problem.value = null;
      undone.value = null;
      phase.value = "one";
    },
    back() {
      phase.value = current.value && phase.value !== "one" ? "one" : "list";
      if (phase.value === "list") current.value = null;
      preview.value = null;
      forgetting.value = [];
      choices.value = {};
      seen.value = [];
    },
    making() {
      problem.value = null;
      phase.value = "make";
    },
    /** Change one conflict's answer and preview again with it. */
    async choose(path: string, choice: Choice) {
      choices.value = { ...choices.value, [path]: choice };
      await look();
    },
    /** The same answer for every conflict on screen. */
    async chooseAll(choice: Choice) {
      const every: Record<string, Choice> = {};
      for (const c of seen.value) every[c.path] = choice;
      choices.value = { ...choices.value, ...every };
      await look();
    },
    async forget(paths: string[]) {
      forgetting.value = paths;
      choices.value = {};
      seen.value = [];
      await look();
    },
    async run() {
      if (!current.value || !preview.value) return;
      problem.value = null;
      rows.value = [];
      leg.value = null;
      ran.value = null;
      phase.value = "running";
      try {
        await start(current.value.name, ask(), preview.value.fingerprint, confirmed.value);
      } catch (e) {
        problem.value = reason(e);
        phase.value = "preview";
      }
    },
    async stop() {
      stopping.value = true;
      try {
        await syncs.stop(false);
      } catch (e) {
        stopping.value = false;
        problem.value = reason(e);
      }
    },
    async stopNow() {
      halting.value = true;
      try {
        await syncs.stop(true);
      } catch (e) {
        halting.value = false;
        problem.value = reason(e);
      }
    },
    async putBack(name: string, plan: number) {
      if (busy.value != null) return;
      busy.value = plan;
      problem.value = null;
      try {
        undone.value = await syncs.putBack(name, plan);
      } catch (e) {
        problem.value = reason(e);
      } finally {
        busy.value = null;
        await loadPast();
      }
    },
    async remove(name: string) {
      await syncs.remove(name);
      current.value = null;
      phase.value = "list";
      await load();
    },
    /** A launch-time run, from the sheet: the preview it was shown with. */
    async runPreviewed(name: string, previewed: SyncPreview, yes: boolean) {
      waiting.value += 1;
      rows.value = [];
      ran.value = null;
      try {
        await start(name, null, previewed.fingerprint, yes);
      } catch (e) {
        waiting.value -= 1;
        problem.value = `${name}: ${reason(e)}`;
      }
    },
  };
}

/** Whether a launch-time run may go without asking: it takes nothing off any
 *  member, needs no yes, and changes nothing that cannot be put back. Replaced
 *  versions are set aside like any other run's, and are fine. */
export function quietlyFine(p: SyncPreview): boolean {
  return p.reversible && !p.refusals.length && p.members.every((m) => m.removing === 0);
}

/** Every sync marked to run when the window opens, previewed, after the
 *  interrupted-work check. Quiet ones that are safe to run go at once; the
 *  rest are handed back for one confirmation. */
export async function launchPreviews(): Promise<{ sync: SyncView; preview: SyncPreview }[]> {
  try {
    await transfers.interrupted();
  } catch {
    // The check is Home's to show; a sync is not held back because it failed.
  }
  const s = useSync();
  let marked: SyncView[];
  try {
    // A sync kept in step is run at launch by the loop that keeps it, not here.
    marked = (await syncs.list()).filter((sync) => sync.launch === "ask" || sync.launch === "quietly");
  } catch {
    return [];
  }
  const asking: { sync: SyncView; preview: SyncPreview }[] = [];
  for (const sync of marked) {
    let previewed: SyncPreview;
    try {
      previewed = await syncs.preview(sync.name, null);
    } catch (e) {
      s.problem.value = `${sync.name} could not be looked at: ${reason(e)}`;
      continue;
    }
    if (previewed.empty) continue;
    if (sync.launch === "quietly" && quietlyFine(previewed)) {
      await s.runPreviewed(sync.name, previewed, false);
    } else {
      asking.push({ sync, preview: previewed });
    }
  }
  return asking;
}
