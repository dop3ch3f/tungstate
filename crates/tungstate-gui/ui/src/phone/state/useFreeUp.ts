// One Free up run: what was asked for, where it has got to, and how it
// ended. Module-level, so the running screen and the tab it came from read
// the same run.
//
// Pause and Stop send the same thing to the engine: finish the photo in
// hand, then stop. They differ only here. A paused run keeps what was asked,
// and Resume asks for it again; photos already on the connection come back
// "already there" and count as checked, so nothing is sent twice.
//
// Clearing comes once, after the run, and only while the app is on screen:
// a run that ends with the phone locked clears when the person is back.

import { ref, shallowRef, watch } from "vue";
import { freeUp } from "../../engine/commands";
import { freeUpEvents, type UnlistenFn } from "../../engine/events";
import type { CheckLevel, FreeUpRan, FreeUpStart, FreeUpTick, Removed } from "../../engine/types";

export type Phase = "idle" | "starting" | "running" | "pausing" | "paused" | "stopping" | "done";

const CHECK_KEY = "tungstate.freeup.check";

const phase = ref<Phase>("idle");
const tick = shallowRef<FreeUpTick | null>(null);
const ran = shallowRef<FreeUpRan | null>(null);
/** The last run asked for, so a paused one can be resumed. */
const asked = shallowRef<FreeUpStart | null>(null);
const problem = ref<string | null>(null);
/** When this stretch of sending began, for the time left. */
const since = ref(0);
/** Asking the phone to remove what was checked, and what it said. */
const clearing = ref(false);
const cleared = shallowRef<Removed | null>(null);
/** The person's check level. Read back unless they chose otherwise. */
const check = ref<CheckLevel>(localStorage.getItem(CHECK_KEY) === "size" ? "size" : "readback");

let listening: Promise<UnlistenFn[]> | null = null;

/** Listen before the first run starts, so its first tick is not missed. */
function attach(): Promise<UnlistenFn[]> {
  listening ??= Promise.all([
    freeUpEvents.progress((t) => {
      tick.value = t;
    }),
    freeUpEvents.done((r) => {
      ran.value = r;
      phase.value = phase.value === "pausing" && !r.problem ? "paused" : "done";
    }),
  ]);
  return listening;
}

async function start(next: FreeUpStart) {
  await attach();
  asked.value = next;
  problem.value = null;
  ran.value = null;
  tick.value = null;
  cleared.value = null;
  phase.value = "starting";
  try {
    await freeUp.start(next);
    since.value = Date.now();
    // A short run can end before its start is even acknowledged.
    if (phase.value === "starting") phase.value = "running";
  } catch (e) {
    problem.value = String(e);
    phase.value = "idle";
  }
}

async function pause() {
  phase.value = "pausing";
  await freeUp.stop();
}

async function stop() {
  // Already stopped: what the pause left is the answer.
  if (phase.value === "paused") {
    phase.value = "done";
    return;
  }
  phase.value = "stopping";
  await freeUp.stop();
}

async function resume() {
  if (asked.value) await start(asked.value);
}

function finish() {
  phase.value = "idle";
  ran.value = null;
  tick.value = null;
  cleared.value = null;
}

async function clear() {
  if (clearing.value) return;
  clearing.value = true;
  try {
    cleared.value = await freeUp.clear();
  } catch (e) {
    problem.value = String(e);
  } finally {
    clearing.value = false;
  }
}

/** Run `then` now if the app is on screen, or as soon as it is back. */
function onScreen(then: () => void) {
  if (document.visibilityState === "visible") return then();
  const back = () => {
    if (document.visibilityState !== "visible") return;
    document.removeEventListener("visibilitychange", back);
    then();
  };
  document.addEventListener("visibilitychange", back);
}

// Every way a run ends with something checked leads here: finished, stopped,
// or stopped after a pause.
watch(phase, (now) => {
  if (now === "done" && ran.value?.checked) onScreen(() => void clear());
});

function setCheck(level: CheckLevel) {
  check.value = level;
  localStorage.setItem(CHECK_KEY, level);
}

/** Seconds left at the pace so far, once there is a pace to go by. */
function secondsLeft(now = Date.now()): number | null {
  const t = tick.value;
  const elapsed = (now - since.value) / 1000;
  if (!t || t.bytes <= 0 || elapsed < 5) return null;
  return ((t.total - t.bytes) / t.bytes) * elapsed;
}

export function useFreeUp() {
  return { phase, tick, ran, asked, problem, check, clearing, cleared, start, pause, stop, resume, finish, clear, setCheck, secondsLeft };
}
