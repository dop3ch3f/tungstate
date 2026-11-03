// The phone's storage as the Files tab shows it: whether the app may use all
// of it, each volume's size, the category tiles, and the folders opened lately.
// Module-level, so Files home and every window agree on the volumes' names.

import { computed, ref, shallowRef } from "vue";
import { phone } from "../../engine/commands";
import type { Categories, PhotoAccess, Volume } from "../../engine/types";
import type { Top } from "../lib/paths";

const RECENT_KEY = "tungstate.phone.recent";
/** How many recent folders Files home lists. */
const RECENT = 3;

const access = ref<PhotoAccess | null>(null);
const volumes = shallowRef<Volume[]>([]);
const categories = shallowRef<Categories | null>(null);
const problem = ref<string | null>(null);
const recent = ref<string[]>(readRecent());

function readRecent(): string[] {
  try {
    const raw = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(raw) ? raw.filter((p): p is string => typeof p === "string").slice(0, RECENT) : [];
  } catch {
    return [];
  }
}

/** Volumes by the names the chips use; connections are tops of their own. */
const tops = computed<Top[]>(() => volumes.value.map((v) => ({ label: v.label, path: v.path })));

async function load() {
  try {
    volumes.value = await phone.volumes();
    access.value = await phone.filesAccess();
    problem.value = null;
  } catch (e) {
    problem.value = String(e);
    return;
  }
  if (access.value === "full") void count();
}

async function count() {
  try {
    categories.value = await phone.categories();
  } catch (e) {
    problem.value = String(e);
  }
}

async function ask() {
  try {
    access.value = await phone.askFiles();
  } catch (e) {
    problem.value = String(e);
    return;
  }
  if (access.value === "full") void count();
}

/** A folder was opened: first in the recent list, once. */
function visited(path: string) {
  recent.value = [path, ...recent.value.filter((p) => p !== path)].slice(0, RECENT);
  localStorage.setItem(RECENT_KEY, JSON.stringify(recent.value));
}

export function useStorage() {
  return { access, volumes, categories, problem, recent, tops, load, count, ask, visited };
}
