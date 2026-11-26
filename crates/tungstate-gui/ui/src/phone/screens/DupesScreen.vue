<!-- Duplicates on this phone: choose where to look, find, then a list of
     groups with previews where the copy that stays is starred and the extras
     are ticked to clear. Clearing sets them aside in a holding folder inside
     the place scanned, so they can be put back, until the holding folder is
     emptied for good. The desktop's own scan and clear-out (useDupes), drawn
     for a phone. Android only for now: an iPhone app cannot see these folders. -->
<script setup lang="ts">
import { computed, onMounted, ref, shallowRef } from "vue";
import { dupes, files } from "../../engine/commands";
import type { DupeCopy, DupeRow, PastRun } from "../../engine/types";
import { ago, bytes, plural } from "../../lib/format";
import { attachDupeStream, useDupes } from "../../state/useDupes";
import { useBack } from "../lib/back";
import { childOf, nameOf, parentOf, placeName } from "../lib/paths";
import { useStorage } from "../state/useStorage";
import PButton from "../ui/PButton.vue";
import PPlaces from "../ui/PPlaces.vue";
import PScreen from "../ui/PScreen.vue";
import PSheet from "../ui/PSheet.vue";
import PThumb from "../ui/PThumb.vue";
import PTrouble from "../ui/PTrouble.vue";

const d = useDupes();
const store = useStorage();
const where = ref<string | null>(null);
const past = shallowRef<PastRun[]>([]);
const said = ref<string | null>(null);
const sheet = ref<"empty" | null>(null);
const busy = ref(false);
/** Groups drawn before "Show more": a web view slows with thousands of rows. */
const shown = ref(60);

onMounted(async () => {
  void attachDupeStream();
  await loadPast();
});
async function loadPast() {
  try {
    past.value = await dupes.past();
  } catch {
    past.value = [];
  }
}
// Clear-outs whose holding folder was emptied. The journal still calls them
// undoable, but there is nothing left to put back, so they are not offered.
const EMPTIED_KEY = "tungstate.phone.emptied";
const emptied = ref<Set<number>>(new Set(JSON.parse(localStorage.getItem(EMPTIED_KEY) ?? "[]") as number[]));
/** The latest clear-out on this phone that can still be put back. */
const last = computed(() => past.value.find((r) => r.undoable && r.root.startsWith("/storage/") && !emptied.value.has(r.plan)) ?? null);

useBack(() => {
  if (d.phase.value === "found" || d.phase.value === "done") return d.again(), true;
  return false;
});

const ORDER = { identical: 0, same: 1, similar: 2 } as const;
const groups = computed<DupeRow[]>(() => [...(d.found.value?.rows ?? [])].filter((r) => !r.folder).sort((a, b) => ORDER[a.claim] - ORDER[b.claim] || b.reclaimable - a.reclaimable));
const ticked = computed(() => d.ticked.value);
const tickedBytes = computed(() => groups.value.flatMap((r) => r.copies).reduce((n, c) => n + (ticked.value.has(c.path) ? c.size : 0), 0));

/** Ticks or unticks one copy, never the last one left in its group. */
function toggle(row: DupeRow, copy: DupeCopy) {
  const on = !ticked.value.has(copy.path);
  if (on && row.copies.filter((c) => !ticked.value.has(c.path)).length <= 1) {
    said.value = "One copy of each always stays. Untick another to clear this one.";
    return;
  }
  said.value = null;
  d.tick(copy.path, on);
}
/** A copy's place on the phone: the scan names copies from where it began. */
const full = (path: string) => (d.root.value ? childOf(d.root.value, path) : path);
const folderOf = (path: string) => {
  const up = parentOf(full(path));
  return up ? placeName(up, store.tops.value) : "";
};
const claimLine = (row: DupeRow) => (row.claim === "identical" ? `${row.copies.length} copies` : row.claim === "same" ? "Look the same" : "Look alike");

async function putBackLast() {
  const run = last.value;
  if (!run) return;
  busy.value = true;
  try {
    const n = await dupes.putBack(run.root, run.plan);
    said.value = `Put ${plural(n, "file")} back where they were.`;
    await loadPast();
  } catch (e) {
    said.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function find() {
  if (!where.value) return;
  shown.value = 60;
  said.value = null;
  await d.look(where.value);
}
async function clear() {
  await d.clear("set-aside");
  await loadPast();
}
/** The holding folder of the place the last clear-out was in. */
const holding = computed(() => (last.value ? `${last.value.root.replace(/\/+$/, "")}/.tungstate-quarantine` : null));
async function empty() {
  const at = holding.value;
  if (!at) return;
  busy.value = true;
  try {
    await files.remove([at]);
    // Every clear-out in that place shared the one holding folder.
    const gone = past.value.filter((r) => r.root === last.value?.root).map((r) => r.plan);
    emptied.value = new Set([...emptied.value, ...gone]);
    localStorage.setItem(EMPTIED_KEY, JSON.stringify([...emptied.value]));
    said.value = "The holding folder is empty. That space is free now.";
    await loadPast();
  } catch (e) {
    said.value = String(e);
  } finally {
    busy.value = false;
    sheet.value = null;
  }
}
const progressLine = computed(() => {
  const p = d.progress.value;
  if (!p) return "Starting…";
  return p.stage === "alike" ? `Comparing pictures · ${p.read.toLocaleString()} looked at` : `${p.looked.toLocaleString()} files looked at · ${bytes(p.bytes)} read`;
});
</script>

<template>
  <!-- Choosing where. -->
  <PScreen v-if="d.phase.value === 'start'" title="Duplicates" :tile="{ art: 'dupes', tint: 'var(--tint-dupes)', light: true }">
    <PTrouble v-if="d.problem.value" :error="d.problem.value" />
    <p v-if="said" class="line">{{ said }}</p>
    <p v-if="d.putBackCount.value !== null" class="line good">Put {{ plural(d.putBackCount.value, "file") }} back where they were.</p>
    <h2>Look in</h2>
    <PPlaces v-model="where" wanted="dupes" />
    <button class="list toggle" role="switch" :aria-checked="d.alsoSimilar.value" @click="d.alsoSimilar.value = !d.alsoSimilar.value">
      <span class="text"><b>Also find look-alikes</b><span class="quiet wrap">Photos that are nearly the same. Never ticked for you.</span></span>
      <span class="switch" :class="{ on: d.alsoSimilar.value }" aria-hidden="true"></span>
    </button>
    <template v-if="last">
      <h2>Last time</h2>
      <div class="list">
        <div class="row">
          <span class="text"><b>{{ ago(last.applied_at) }} · {{ plural(last.files, "copy", "copies") }}, {{ bytes(last.bytes) }}</b><span class="quiet">In the holding folder in {{ placeName(last.root, store.tops.value) }}</span></span>
        </div>
        <div class="pair pad">
          <PButton quiet :busy="busy" @press="putBackLast">Put back</PButton>
          <PButton quiet @press="sheet = 'empty'">Empty it</PButton>
        </div>
      </div>
    </template>
    <template #act>
      <PButton tint="var(--tint-dupes)" :disabled="!where" @press="find">Find duplicates</PButton>
    </template>
    <PSheet v-if="sheet === 'empty' && last" title="Empty the holding folder?" :says="`${plural(last.files, 'copy', 'copies')}, ${bytes(last.bytes)}, are deleted for good and can't be put back.`" @close="sheet = null">
      <PButton tint="var(--bad)" :busy="busy" @press="empty">Delete them for good</PButton>
      <PButton quiet @press="sheet = null">Keep them</PButton>
    </PSheet>
  </PScreen>

  <!-- Looking. -->
  <PScreen v-else-if="d.phase.value === 'scanning'" title="Looking for duplicates">
    <p class="line">{{ progressLine }}</p>
    <p v-if="d.progress.value?.path" class="line path">{{ nameOf(d.progress.value.path) }}</p>
    <template #act>
      <PButton quiet :busy="d.stopping.value" @press="d.stop()">Stop</PButton>
    </template>
  </PScreen>

  <!-- What it found. -->
  <PScreen v-else-if="d.phase.value === 'found' || d.phase.value === 'clearing'" :title="groups.length ? plural(ticked.size, 'copy', 'copies') : 'No duplicates'" back="Duplicates" @back="d.again()">
    <p v-if="groups.length" class="line">{{ bytes(tickedBytes) }}. ★ is the copy that stays. Tap a copy to change what's cleared.</p>
    <p v-else class="line">Nothing in {{ where ? placeName(where, store.tops.value) : "there" }} is a copy of anything else.</p>
    <p v-if="said" class="line">{{ said }}</p>
    <PTrouble v-if="d.problem.value" :error="d.problem.value" />
    <div v-for="row in groups.slice(0, shown)" :key="row.id" class="list group">
      <div class="gh"><b>{{ row.copies.find((c) => c.keep)?.name ?? row.copies[0]?.name }}</b><span class="num">{{ claimLine(row) }}</span></div>
      <button v-for="c in row.copies" :key="c.path" class="copy" :class="{ sel: ticked.has(c.path), kept: !ticked.has(c.path) }" @click="toggle(row, c)">
        <span v-if="!ticked.has(c.path)" class="star" aria-label="Stays">★</span>
        <span v-else class="check on" aria-label="Cleared">✓</span>
        <PThumb :name="c.name" :path="full(c.path)" :size="56" />
        <span class="text"><b>{{ nameOf(parentOf(full(c.path)) ?? "") || c.name }}</b><span class="quiet">{{ bytes(c.size) }} · {{ folderOf(c.path) }}</span></span>
      </button>
    </div>
    <button v-if="groups.length > shown" class="more" @click="shown += 200">Show {{ Math.min(200, groups.length - shown) }} more</button>
    <template v-if="groups.length" #act>
      <p class="say">They wait in a holding folder until you empty it, so you can put them back.</p>
      <PButton tint="var(--tint-dupes)" :busy="d.phase.value === 'clearing'" :disabled="!ticked.size" @press="clear">Clear {{ plural(ticked.size, "copy", "copies") }}</PButton>
    </template>
  </PScreen>

  <!-- Done. -->
  <PScreen v-else title="Cleared" back="Duplicates" @back="d.again()">
    <p class="line good">{{ plural(d.cleared.value?.files ?? 0, "copy", "copies") }}, {{ bytes(d.cleared.value?.bytes ?? 0) }}, are in the holding folder.</p>
    <p v-if="d.cleared.value?.dropped" class="line">{{ plural(d.cleared.value.dropped, "group") }} turned out not to be identical when checked, so they were left alone.</p>
    <PTrouble v-if="d.cleared.value?.failed.length" :error="`${plural(d.cleared.value.failed.length, 'file')} could not be moved: ${d.cleared.value.failed.slice(0, 3).join(', ')}`" />
    <p class="line">Empty the holding folder from Duplicates to free the space for good.</p>
    <template #act>
      <div class="pair">
        <PButton v-if="d.cleared.value?.reversible" quiet :busy="d.puttingBack()" @press="d.putBack()">Put them back</PButton>
        <PButton tint="var(--tint-dupes)" @press="d.again(); loadPast()">Done</PButton>
      </div>
    </template>
  </PScreen>
</template>

<style scoped>
h2 { margin: 18px 0 8px; font-size: 15px; font-weight: 800; }
h2:first-of-type { margin-top: 0; }
.line { margin: 0 2px 12px; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.good { color: var(--ok); font-weight: 700; }
.list { overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.toggle { display: flex; align-items: center; gap: 12px; width: 100%; margin-top: 14px; padding: 12px 14px; font: inherit; text-align: left; color: var(--text); }
.row { display: flex; align-items: center; gap: 12px; padding: 12px 14px; }
.pad { padding: 0 14px 14px; }
.pair { display: flex; gap: 12px; }
.pair > * { flex: 1; }
.text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.text b { font-size: 16px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.quiet { font-size: 14px; color: var(--text-quiet); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.wrap { white-space: normal; }
.switch { position: relative; flex: none; width: 50px; height: 30px; background: var(--panel); border: 2px solid var(--edge); border-radius: 99px; }
.switch::after { content: ""; position: absolute; top: 3px; left: 3px; width: 20px; height: 20px; border-radius: 99px; background: var(--edge); }
.switch.on { background: var(--tint-dupes); }
.switch.on::after { left: 23px; background: #ffffff; }
.group { margin-bottom: 12px; }
.gh { display: flex; align-items: center; gap: 12px; padding: 10px 14px; font-size: 15px; background: var(--rail); }
.gh b { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.num { flex: none; font-weight: 800; }
.copy { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 80px; padding: 10px 14px; font: inherit; text-align: left; color: var(--text); background: var(--panel); border: 0; border-top: 2px solid var(--edge); }
.copy.sel { background: color-mix(in srgb, var(--tint-drain) 12%, var(--panel)); }
.copy.kept { background: color-mix(in srgb, var(--tint-dupes) 10%, var(--panel)); }
.star, .check { flex: none; width: 30px; height: 30px; display: grid; place-items: center; font-weight: 900; border: 2px solid var(--edge); border-radius: 6px; }
.star { font-size: 17px; color: #ffffff; background: var(--tint-dupes); }
.check.on { color: #ffffff; background: var(--tint-drain); }
.more { width: 100%; min-height: 48px; margin-bottom: 12px; font: inherit; font-size: 15px; font-weight: 800; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.say { margin: 0 0 10px; font-size: 14px; text-align: center; color: var(--text-quiet); }
</style>
