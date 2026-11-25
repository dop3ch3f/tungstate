// The Android back button. Whatever is on top steps back first: a sheet
// closes, then the viewer, then selection ends, then the folder goes up a
// level. Each registers a step while it is on screen; the newest one that
// can step back does, and the rest are left alone.

import { onMounted, onUnmounted } from "vue";

/** Answers true when it stepped back, false when it had nothing to undo. */
export type Step = () => boolean;

const steps: Step[] = [];

/** Adds a step; the returned function takes it away again. */
export function addStep(step: Step): () => void {
  steps.push(step);
  return () => {
    const at = steps.lastIndexOf(step);
    if (at >= 0) steps.splice(at, 1);
  };
}

/** One press of back. False when nothing stepped back: the app may close. */
export function stepBack(): boolean {
  for (let i = steps.length - 1; i >= 0; i--) if (steps[i]!()) return true;
  return false;
}

/** A step for as long as the calling component is on screen. */
export function useBack(step: Step): void {
  let remove = () => {};
  onMounted(() => (remove = addStep(step)));
  onUnmounted(() => remove());
}
