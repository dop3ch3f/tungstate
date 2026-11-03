// The file windows: one, or two to swipe between, as ES File Explorer has.
// Each holds where it is, how it lists, and what is ticked there.
// Module-level, so the tab bar can step aside while a window is selecting.

import { computed, reactive, ref } from "vue";
import type { Category } from "../../engine/types";

/** Where a window is: Files home, a folder, or one category tile's files. */
export type Place =
  | { kind: "home" }
  | { kind: "folder"; path: string }
  | { kind: "category"; category: Category };

export type Sort = "newest" | "oldest" | "name" | "largest";
export type View = "list" | "grid";

export interface Win {
  place: Place;
  sort: Sort;
  view: View;
  /** Ticked, by location. Ticking anything puts the window in selection. */
  picked: Set<string>;
}

/** Two at most: a phone screen shows one, and the second is for a copy or a
 *  move between two places, which is the only reason ES users open more. */
const MAX = 2;
const VIEW_KEY = "tungstate.phone.view";

const fresh = (place: Place = { kind: "home" }): Win =>
  reactive({ place, sort: "newest", view: (localStorage.getItem(VIEW_KEY) as View | null) ?? "list", picked: new Set<string>() });

const wins = ref<Win[]>([fresh()]);
const active = ref(0);

const win = computed(() => wins.value[active.value]!);
/** The window is choosing files, so the tab bar gives way to the actions. */
const selecting = computed(() => win.value.picked.size > 0);

function go(place: Place) {
  win.value.place = place;
  win.value.picked = new Set();
}

function toggle(path: string) {
  const next = new Set(win.value.picked);
  if (!next.delete(path)) next.add(path);
  win.value.picked = next;
}

function pickAll(paths: string[]) {
  win.value.picked = new Set(paths);
}

function clearPicks() {
  win.value.picked = new Set();
}

/** Opens the second window, or switches to it if it is open. */
function second(place: Place = { kind: "home" }) {
  if (wins.value.length < MAX) wins.value = [...wins.value, fresh(place)];
  else wins.value[1]!.place = place;
  active.value = wins.value.length - 1;
}

function close(index: number) {
  if (wins.value.length === 1) return;
  wins.value = wins.value.filter((_, i) => i !== index);
  active.value = Math.min(active.value, wins.value.length - 1);
}

function switchTo(index: number) {
  if (index >= 0 && index < wins.value.length) active.value = index;
}

/** One list/grid choice for every window, remembered: the user's own call. */
function setView(view: View) {
  win.value.view = view;
  localStorage.setItem(VIEW_KEY, view);
}

export function useWindows() {
  return { wins, active, win, selecting, go, toggle, pickAll, clearPicks, second, close, switchTo, setView, max: MAX };
}
