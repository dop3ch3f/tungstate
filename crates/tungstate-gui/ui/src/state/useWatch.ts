// What the watcher has done while this window has been open.
//
// Module-level refs, like the other state modules. The listener is attached
// once, from the frame, before anything that can throw.

import { ref, shallowRef } from "vue";
import { watcher } from "../engine/commands";
import { watchEvents, type UnlistenFn } from "../engine/events";
import type { Notice, WatchState } from "../engine/types";

const on = ref(true);
const running = ref(false);
const watching = ref(0);
const sweeping = ref(0);
const recent = shallowRef<Notice[]>([]);

/** How many of the newest things are worth showing on Home. */
const SHOWN = 6;

let attached = false;
const unlisten: UnlistenFn[] = [];

export async function attachWatchStream() {
  if (attached) return;
  attached = true;
  try {
    unlisten.push(
      await watchEvents.noticed((notice) => {
        // The "nothing to do" of an hourly sweep is state rather than news:
        // on screen it would bury the two lines that matter under one per
        // folder per hour.
        if (notice.kind === "settled") return;
        running.value = true;
        // Read the list back rather than pushing onto it. Which notices
        // replace which is a rule, and a rule kept in two places is a rule
        // that drifts; the engine owns it.
        void load();
      }),
    );
  } catch {
    // A watcher that cannot be heard is still watching; the screen says less.
  }
}

export function detachWatchStream() {
  for (const off of unlisten.splice(0)) off();
  attached = false;
}

function take(state: WatchState) {
  on.value = state.on;
  running.value = state.running;
  watching.value = state.watching;
  sweeping.value = state.sweeping;
  recent.value = state.recent.filter((notice) => notice.kind !== "settled").slice(0, SHOWN);
}

async function load() {
  try {
    take(await watcher.state());
  } catch {
    on.value = false;
  }
}

async function set(wanted: boolean) {
  on.value = wanted;
  try {
    await watcher.set(wanted);
  } catch {
    // Put the switch back where it was rather than lying about the state.
    await load();
    return;
  }
  if (!wanted) {
    running.value = false;
    recent.value = [];
  }
  await load();
}

export function useWatch() {
  return { on, running, watching, sweeping, recent, load, set };
}
