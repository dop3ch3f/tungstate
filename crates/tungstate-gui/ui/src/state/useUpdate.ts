// Updates from inside the app.
//
// Checks the latest tagged release at launch and once a day, and says so
// quietly when there is a newer one. Nothing is fetched or installed until the
// person asks, and the restart waits for anything that is writing files: a
// transfer, a sync, a tidy, or duplicates being cleared.

import { ref, shallowRef } from "vue";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { updates } from "../engine/commands";
import { useFolders } from "./useFolders";
import { useDupes } from "./useDupes";

export type Step = "idle" | "fetching" | "waiting" | "installing";
/** What the restart is waiting for, engine's or the window's own. */
export type Waiting = "transfer" | "sync" | "tidy" | "clearing";

const DAY = 24 * 60 * 60 * 1000;

const ready = shallowRef<Update | null>(null);
const checking = ref(false);
const checkedAt = ref<number | null>(null);
const step = ref<Step>("idle");
const waitingFor = ref<Waiting | null>(null);
const got = ref(0);
const total = ref<number | null>(null);
const problem = ref<string | null>(null);
/** The sheet that shows the release and applies it. */
const showing = ref(false);

async function look() {
  if (checking.value || step.value !== "idle") return;
  checking.value = true;
  problem.value = null;
  try {
    const found = await check();
    // A second look finds the same release as a new resource; keep one.
    if (ready.value && found?.version === ready.value.version) await found.close();
    else ready.value = found;
    checkedAt.value = Date.now();
  } catch (e) {
    problem.value = String(e);
  } finally {
    checking.value = false;
  }
}

/** Work the window itself is doing, that a restart would cut short. */
function busyHere(): Waiting | null {
  if (useFolders().busy.value) return "tidy";
  if (useDupes().phase.value === "clearing") return "clearing";
  return null;
}

const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Fetch the update, wait for anything writing, install it, restart. `every`
 *  is how often to ask again while something is running. */
async function apply({ every = 5000 } = {}) {
  const update = ready.value;
  if (!update || step.value !== "idle") return;
  problem.value = null;
  step.value = "fetching";
  got.value = 0;
  total.value = null;
  let cleared = false;
  try {
    await update.download((e) => {
      if (e.event === "Started") total.value = e.data.contentLength ?? null;
      else if (e.event === "Progress") got.value += e.data.chunkLength;
    });
    for (;;) {
      const busy = busyHere() ?? (await updates.readyToRestart());
      if (!busy) break;
      waitingFor.value = busy;
      step.value = "waiting";
      await pause(every);
    }
    cleared = true;
    waitingFor.value = null;
    step.value = "installing";
    // On Windows the installer ends this process itself; elsewhere the new
    // version is in place and this starts it.
    await update.install();
    await relaunch();
  } catch (e) {
    problem.value = String(e);
    step.value = "idle";
    waitingFor.value = null;
    if (cleared) await updates.notRestarting().catch(() => {});
  }
}

let timer: ReturnType<typeof setInterval> | null = null;

/** Look now and once a day after, for as long as the window is open. */
export function startLooking() {
  void look();
  timer ??= setInterval(() => void look(), DAY);
}

export function useUpdate() {
  return { ready, checking, checkedAt, step, waitingFor, got, total, problem, showing, look, apply };
}
