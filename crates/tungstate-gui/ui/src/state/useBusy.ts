// Waiting on the engine, for a screen with more than one thing to wait on.
//
// Every engine call runs off the main thread, so the window stays live while
// one works and a second click would start it again. `run` takes a key (a
// plan id, a link name, or "save"), refuses a second call under a key that is
// already running, and lets the button draw its busy state before the call.

import { ref } from "vue";

/** Wait for the screen to be drawn, but never for long.
 *
 *  Two animation frames is "Vue has flushed and the browser has painted". A
 *  hidden or minimised window stops sending frames altogether, so the wait is
 *  raced with a timer: without it, a call started and then minimised would sit
 *  on a promise that never settles and never be made. */
export const painted = () =>
  new Promise<void>((resolve) => {
    const done = () => resolve();
    requestAnimationFrame(() => requestAnimationFrame(done));
    setTimeout(done, 80);
  });

export function useBusy<K extends string | number = string>() {
  // Replaced rather than mutated, so a template reading it re-renders.
  const running = ref<ReadonlySet<K>>(new Set());
  const problem = ref<string | null>(null);

  async function run<T>(key: K, work: () => Promise<T>): Promise<T | null> {
    if (running.value.has(key)) return null;
    running.value = new Set([...running.value, key]);
    problem.value = null;
    await painted();
    try {
      return await work();
    } catch (e) {
      problem.value = String(e);
      return null;
    } finally {
      const rest = new Set(running.value);
      rest.delete(key);
      running.value = rest;
    }
  }

  return {
    run,
    problem,
    /** Whether `key`, or with no key anything at all, is running. */
    busy: (key?: K) => (key === undefined ? running.value.size > 0 : running.value.has(key)),
  };
}
