// The file windows, as tabs along the top the way ES File Explorer has them:
// each storage and connection in its own, copy in one, paste in another.
// Each holds where it is, how it lists, and what is ticked there. The first
// is home: it never closes, and only the phone's own storage opens in it.
// Module-level, so every screen sees the same tabs.

import { computed, reactive, ref } from "vue";
import type { Category } from "../../engine/types";
import { storedView, type View } from "../lib/views";

/** Where a window is: Files home, a folder, or one category tile's files. */
export type Place =
  | { kind: "home" }
  | { kind: "folder"; path: string }
  | { kind: "category"; category: Category };

export type Sort = "newest" | "oldest" | "name" | "largest";
export type { View };

export interface Win {
  place: Place;
  sort: Sort;
  view: View;
  /** Ticked, by location. Ticking anything puts the window in selection. */
  picked: Set<string>;
  /** How far down each folder was scrolled, so coming back up a level lands
   *  where you left it, as ES does. */
  scrolled: Map<string, number>;
}

/** Enough for this phone, a card and a few connections; past that the strip
 *  stops fitting a phone's width and the tabs stop being findable. */
const MAX = 5;
const VIEW_KEY = "tungstate.phone.view";
const HIDDEN_KEY = "tungstate.phone.hidden";


const fresh = (place: Place = { kind: "home" }): Win =>
  reactive({ place, sort: "newest", view: storedView(localStorage.getItem(VIEW_KEY)), picked: new Set<string>(), scrolled: new Map<string, number>() });

const wins = ref<Win[]>([fresh()]);
/** Names starting with a dot, shown or not, in every window. */
const hidden = ref(localStorage.getItem(HIDDEN_KEY) === "1");

function setHidden(on: boolean) {
  hidden.value = on;
  localStorage.setItem(HIDDEN_KEY, on ? "1" : "0");
}
const active = ref(0);

const win = computed(() => wins.value[active.value]!);
/** The window is choosing files, so the bottom tabs give way to the actions. */
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

/** The tab that is always there, first in the strip. */
export const HOME = 0;

/** Whether a window is somewhere inside `top`: a folder (a volume, or one on
 *  a connection), or a whole connection (`Area51:`), anywhere down it. */
function within(at: Place, top: string): boolean {
  if (at.kind !== "folder") return false;
  const root = top.replace(/\/+$/, "");
  // `Area51:` is followed directly by its folders, `Area51:Photos`.
  return at.path === root || at.path.startsWith(root.endsWith(":") ? root : `${root}/`);
}

/** Opens somewhere off the phone's own storage (a connection, a card) in a
 *  tab of its own, so home stays home. A blank tab opened with + takes it, as
 *  a browser's new tab page does, even where another tab is already there:
 *  two tabs on one place is how to copy between two of its folders. From
 *  home or a tab in use: the tab already in that place, else a new tab, else
 *  (all open) the last one gives way, never home. */
function openTab(place: Place) {
  if (active.value !== HOME && win.value.place.kind === "home") return go(place);
  const top = place.kind === "folder" ? place.path : null;
  const there = top === null ? -1 : wins.value.findIndex((x, i) => i !== HOME && within(x.place, top));
  if (there >= 0) return void (active.value = there);
  if (wins.value.length < MAX) {
    wins.value = [...wins.value, fresh(place)];
    active.value = wins.value.length - 1;
    return;
  }
  if (active.value === HOME) active.value = wins.value.length - 1;
  go(place);
}

/** Sends every tab inside `top` back to it: after a connection's folder
 *  changes, the folders those tabs were in mean something else. */
function rehome(top: string) {
  for (const x of wins.value) if (within(x.place, top)) x.place = { kind: "folder", path: top };
}

/** A new tab on Files home, as ES's "+" opens one. */
function newTab() {
  if (wins.value.length >= MAX) return;
  wins.value = [...wins.value, fresh()];
  active.value = wins.value.length - 1;
}

/** Older name for opening a place in a tab of its own; Free up and the
 *  screenshot harness call it. */
const second = (place: Place = { kind: "home" }) => openTab(place);

function close(index: number) {
  if (index === HOME || index < 0 || index >= wins.value.length) return;
  wins.value = wins.value.filter((_, i) => i !== index);
  // A tab before the current one going shifts the current one down by one.
  if (index < active.value) active.value -= 1;
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
  return { wins, active, win, selecting, hidden, setHidden, go, toggle, pickAll, clearPicks, openTab, newTab, rehome, second, close, switchTo, setView, max: MAX };
}
