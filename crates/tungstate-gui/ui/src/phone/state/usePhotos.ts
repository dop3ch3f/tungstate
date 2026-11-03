// The photo library as Free up space sees it: whether the app may look, what
// the filter is, what it found, and which of those the person unticked.
//
// Module-level, so the findings and the run that follows them are one list.
// The run lists the library again itself; only what was unticked is sent
// back, so nothing here can add a photo the filter did not find.

import { computed, ref, shallowRef } from "vue";
import { freeUp, photos } from "../../engine/commands";
import type { PhotoAccess, PhotoAsset, PhotoFilter } from "../../engine/types";
import { byYear } from "../lib/findings";

/** The kinds and ages the filter offers, as the screens word them. */
export const KINDS: { id: PhotoFilter["kind"]; label: string }[] = [
  { id: "all", label: "All" },
  { id: "videos", label: "Videos" },
  { id: "screenshots", label: "Screenshots" },
];
export const AGES: { days: number; label: string }[] = [
  { days: 30, label: "1 month" },
  { days: 90, label: "3 months" },
  { days: 365, label: "1 year" },
  { days: 0, label: "Any" },
];

const access = ref<PhotoAccess | null>(null);
/** The approved default: everything older than three months. */
const filter = ref<PhotoFilter>({ kind: "all", olderThanDays: 90 });
const findings = shallowRef<PhotoAsset[]>([]);
/** Unticked, by id. Favourites start here: they stay unless ticked. */
const unticked = ref<Set<string>>(new Set());
const loading = ref(false);
const problem = ref<string | null>(null);

const canRead = computed(() => access.value === "full" || access.value === "limited");
const years = computed(() => byYear(findings.value, unticked.value));
/** What would be moved, as ticked. */
const chosen = computed(() =>
  years.value.reduce((t, g) => ({ count: t.count + g.ticked, bytes: t.bytes + g.tickedBytes }), { count: 0, bytes: 0 }),
);

async function find() {
  if (!canRead.value) return;
  loading.value = true;
  try {
    const found = await freeUp.findings(filter.value);
    findings.value = found;
    unticked.value = new Set(found.filter((a) => a.favourite).map((a) => a.id));
    problem.value = null;
  } catch (e) {
    problem.value = String(e);
  } finally {
    loading.value = false;
  }
}

/** Find out whether the app may look, and look if it may. */
async function load() {
  try {
    access.value = await photos.access();
  } catch (e) {
    problem.value = String(e);
    return;
  }
  await find();
}

async function ask() {
  try {
    access.value = await photos.ask();
  } catch (e) {
    problem.value = String(e);
    return;
  }
  await find();
}

async function setFilter(next: PhotoFilter) {
  filter.value = next;
  await find();
}

function toggle(id: string) {
  const next = new Set(unticked.value);
  if (!next.delete(id)) next.add(id);
  unticked.value = next;
}

/** Tick or untick a whole year: all of it, unless all of it is ticked. */
function toggleYear(year: number) {
  const group = years.value.find((g) => g.year === year);
  if (!group) return;
  const next = new Set(unticked.value);
  const all = group.ticked === group.items.length;
  for (const a of group.items) {
    if (all) next.add(a.id);
    else next.delete(a.id);
  }
  unticked.value = next;
}

export function usePhotos() {
  return { access, filter, findings, unticked, years, chosen, loading, problem, canRead, load, ask, setFilter, toggle, toggleYear };
}
