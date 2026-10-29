// The photo library as Free up space sees it: whether the app may look, what
// the filter is, what it found, and which years and favourites are left out.
//
// Module-level, like the desktop stores, so Home's headline and Free up's
// list are one set of numbers and cannot disagree.

import { computed, ref, shallowRef } from "vue";
import { photos } from "../../engine/commands";
import type { PhotoAccess, PhotoFilter, PhotoSummary } from "../../engine/types";

const access = ref<PhotoAccess | null>(null);
/** The approved default: everything older than three months. */
const filter = ref<PhotoFilter>({ kind: "all", olderThanDays: 90 });
const summary = shallowRef<PhotoSummary | null>(null);
/** Years taken out of the run, by year. Everything else is in. */
const left = ref<Set<number>>(new Set());
/** Favourites stay on the phone unless the person puts them in. */
const favourites = ref(false);
const loading = ref(false);
const problem = ref<string | null>(null);

const canRead = computed(() => access.value === "full" || access.value === "limited");

/** What would be freed with the years and favourites as chosen. */
const chosen = computed(() => {
  const s = summary.value;
  if (!s) return { count: 0, bytes: 0 };
  let count = 0;
  let bytes = 0;
  for (const y of s.years) {
    if (left.value.has(y.year)) continue;
    count += y.count;
    bytes += y.bytes;
  }
  if (favourites.value) {
    count += s.favourites.count;
    bytes += s.favourites.bytes;
  }
  return { count, bytes };
});

async function count() {
  if (!canRead.value) return;
  loading.value = true;
  try {
    summary.value = await photos.summary(filter.value);
    problem.value = null;
  } catch (e) {
    problem.value = String(e);
  } finally {
    loading.value = false;
  }
}

/** Find out whether the app may look, and count if it may. */
async function load() {
  try {
    access.value = await photos.access();
  } catch (e) {
    problem.value = String(e);
    return;
  }
  await count();
}

async function ask() {
  try {
    access.value = await photos.ask();
  } catch (e) {
    problem.value = String(e);
    return;
  }
  await count();
}

async function setFilter(next: PhotoFilter) {
  filter.value = next;
  left.value = new Set();
  await count();
}

function toggleYear(year: number) {
  const next = new Set(left.value);
  if (!next.delete(year)) next.add(year);
  left.value = next;
}

export function usePhotos() {
  return { access, filter, summary, left, favourites, loading, problem, canRead, chosen, load, ask, setFilter, toggleYear };
}
