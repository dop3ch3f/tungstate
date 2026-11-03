<!-- Free up space: the old photos and videos it found, as a file list you
     untick from, grouped by year; then a second window on a connection to
     choose the folder they go to. Matches docs/mobile/approved-files/
     (findings, choosefolder). Favourites start unticked. Landing starts the
     run, which takes over the screen (FreeingScreen). -->
<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import type { CheckLevel, PhotoAsset, PhotoFilter } from "../../engine/types";
import { bytes, plural, when } from "../../lib/format";
import Pixels from "../../ui/Pixels.vue";
import { useConnections } from "../../state/useConnections";
import { usePhoneNav } from "../nav";
import FilesHome from "../files/FilesHome.vue";
import FileWindow from "../files/FileWindow.vue";
import { yearState } from "../lib/findings";
import { connectionOf } from "../lib/paths";
import { useClipboard } from "../state/useClipboard";
import { AGES, KINDS, usePhotos } from "../state/usePhotos";
import { useFreeUp } from "../state/useFreeUp";
import { useWindows } from "../state/useWindows";
import PScreen from "../ui/PScreen.vue";
import PTile from "../ui/PTile.vue";
import PButton from "../ui/PButton.vue";
import PSheet from "../ui/PSheet.vue";

const lib = usePhotos();
const run = useFreeUp();
const conns = useConnections();
const nav = usePhoneNav();
const board = useClipboard();
const w = useWindows();
onMounted(() => {
  void lib.load();
  void conns.load();
});

const sheet = ref<"filter" | null>(null);
const targets = computed(() => conns.all.value.filter((c) => c.scheme !== "fs"));
/** Choosing the folder: window 2 is up, with the findings on the clipboard. */
const choosing = computed(() => !!board.clip.value?.land);

const ageLabel = computed(() => {
  const age = AGES.find((a) => a.days === lib.filter.value.olderThanDays)!;
  return age.days ? `Older than ${age.label}` : "Any age";
});
const kindLabel = computed(() => (lib.filter.value.kind === "all" ? "Photos and videos" : KINDS.find((k) => k.id === lib.filter.value.kind)!.label));
const pick = (next: Partial<PhotoFilter>) => void lib.setFilter({ ...lib.filter.value, ...next });

// Each level says what it costs, so the choice is an informed one.
const CHECKS: { id: CheckLevel; label: string; says: string }[] = [
  { id: "readback", label: "Read back", says: "Reads each copy back from the connection. Slower, and the only check that proves the bytes there are right." },
  { id: "size", label: "Size only", says: "Only compares sizes. Faster, but a copy damaged on the way would not be caught." },
];

/** Thousands of rows would choke a phone's web view, so each year shows its
 *  first rows until asked for the rest. Ticking a year still covers it all. */
const FIRST = 60;
const opened = ref<Set<number>>(new Set());
const rowsOf = (year: number, items: PhotoAsset[]) => (opened.value.has(year) ? items : items.slice(0, FIRST));
const openYear = (year: number) => (opened.value = new Set([...opened.value, year]));

const ART = { photo: "photo", video: "video", screenshot: "photo" } as const;
const TINT = { photo: "var(--raw-kind-image)", video: "var(--raw-kind-video)", screenshot: "var(--raw-kind-doc)" } as const;
const sub = (a: PhotoAsset) => [bytes(a.size), when(a.takenMs), a.favourite ? "Favourite" : null].filter(Boolean).join(" · ");

/** Hand the findings to the clipboard and open window 2 on a connection. */
function chooseWhere() {
  const first = targets.value[0];
  if (!first) return nav.go("connections");
  const { count, bytes: size } = lib.chosen.value;
  board.hold({
    op: "move",
    what: plural(count, "item"),
    bytes: size,
    from: "Free up",
    paths: [],
    land: land,
  });
  w.second({ kind: "folder", path: `${first.name}:` });
}

/** "Move here" in window 2: the run, into that folder on that connection. */
async function land(destination: string) {
  const connection = connectionOf(destination);
  if (!connection) throw new Error("Choose a folder on a connection; Free up moves them off this phone.");
  await run.start({
    connection,
    folder: destination.slice(connection.length + 1),
    filter: lib.filter.value,
    years_left_out: [],
    // Favourites are in the findings like any photo; unticked ones are
    // left out by id with the rest.
    favourites: true,
    left_out: [...lib.unticked.value],
    verify: run.check.value,
  });
  if (run.problem.value) throw new Error(run.problem.value);
}
</script>

<template>
  <!-- Window 2, choosing the folder; the clipboard bar says what is held. -->
  <template v-if="choosing">
    <FilesHome v-if="w.win.value.place.kind === 'home'" />
    <FileWindow v-else />
  </template>

  <PScreen v-else title="Free up space">
    <div v-if="lib.access.value === 'ask'" class="card">
      <PTile art="drain" tint="var(--tint-drain)" light :size="48" />
      <b>Let Tungstate see your photos</b>
      <p>To find what can be moved off this phone. Nothing leaves it until you choose where it goes.</p>
    </div>
    <div v-else-if="lib.access.value === 'denied'" class="card">
      <PTile art="drain" tint="var(--tint-drain)" light :size="48" />
      <b>Photo access is turned off</b>
      <p>Turn it on in Settings › Apps › Tungstate › Permissions › Photos and videos, then come back.</p>
    </div>

    <template v-else-if="lib.canRead.value">
      <p v-if="lib.loading.value || lib.findings.value.length" class="line">
        <template v-if="lib.loading.value">Looking…</template>
        <template v-else>{{ plural(lib.findings.value.length, "photo or video", "photos and videos") }}, {{ bytes(lib.findings.value.reduce((n, a) => n + a.size, 0)) }}. Untick any you want to keep on the phone.</template>
      </p>
      <p v-if="lib.access.value === 'limited'" class="line">Tungstate sees only the photos you picked.</p>
      <div class="bar">
        <div class="chips">
          <button class="chip on" @click="sheet = 'filter'">{{ ageLabel }} ▾</button>
          <button class="chip" @click="sheet = 'filter'">{{ kindLabel }} ▾</button>
        </div>
        <div class="seg" role="group" aria-label="View">
          <button :class="{ on: w.win.value.view === 'list' }" aria-label="List" @click="w.setView('list')"><Pixels of="list" :size="18" /></button>
          <button :class="{ on: w.win.value.view === 'grid' }" aria-label="Grid" @click="w.setView('grid')"><Pixels of="grid" :size="18" /></button>
        </div>
      </div>

      <p v-if="!lib.loading.value && !lib.findings.value.length" class="line">Nothing on this phone matches. Try another age or kind.</p>
      <div v-for="g in lib.years.value" :key="g.year" class="group">
        <button class="year" @click="lib.toggleYear(g.year)">
          <span class="check" :class="yearState(g)" aria-hidden="true">{{ yearState(g) === "some" ? "–" : "✓" }}</span>
          <b class="grow">{{ g.year }} · {{ g.ticked === g.items.length ? g.items.length.toLocaleString() : `${g.ticked.toLocaleString()} of ${g.items.length.toLocaleString()}` }}</b>
          <span class="num">{{ bytes(g.tickedBytes) }}</span>
        </button>
        <div :class="{ grid: w.win.value.view === 'grid' }">
          <button v-for="a in rowsOf(g.year, g.items)" :key="a.id" class="item" @click="lib.toggle(a.id)">
            <span class="check" :class="{ all: !lib.unticked.value.has(a.id) }" aria-hidden="true">✓</span>
            <PTile :art="ART[a.kind]" :tint="TINT[a.kind]" :size="w.win.value.view === 'grid' ? 56 : 44" />
            <span class="text"><b>{{ a.name }}</b><span class="quiet">{{ sub(a) }}</span></span>
          </button>
        </div>
        <button v-if="g.items.length > FIRST && !opened.has(g.year)" class="more" @click="openYear(g.year)">
          Show all {{ g.items.length.toLocaleString() }} from {{ g.year }}
        </button>
      </div>
    </template>

    <p v-if="lib.problem.value" class="line bad">{{ lib.problem.value }}</p>
    <p v-if="run.problem.value" class="line bad">{{ run.problem.value }}</p>

    <PSheet v-if="sheet === 'filter'" title="What to look for" @close="sheet = null">
      <b class="label">Kind</b>
      <div class="seg wide">
        <button v-for="k in KINDS" :key="k.id" :class="{ on: lib.filter.value.kind === k.id }" @click="pick({ kind: k.id })">{{ k.label }}</button>
      </div>
      <b class="label">Older than</b>
      <div class="seg wide">
        <button v-for="a in AGES" :key="a.days" :class="{ on: lib.filter.value.olderThanDays === a.days }" @click="pick({ olderThanDays: a.days })">{{ a.label }}</button>
      </div>
      <b class="label">Check each one</b>
      <div class="seg wide">
        <button v-for="c in CHECKS" :key="c.id" :class="{ on: run.check.value === c.id }" @click="run.setCheck(c.id)">{{ c.label }}</button>
      </div>
      <p class="says">{{ CHECKS.find((c) => c.id === run.check.value)!.says }}</p>
      <PButton @press="sheet = null">{{ lib.loading.value ? "Looking…" : `Show ${plural(lib.findings.value.length, "item")}` }}</PButton>
    </PSheet>

    <template #act>
      <PButton v-if="lib.access.value === 'ask'" @press="lib.ask()">Allow access</PButton>
      <PButton
        v-else-if="lib.canRead.value"
        :disabled="!lib.chosen.value.count || lib.loading.value"
        :busy="run.phase.value === 'starting'"
        @press="chooseWhere"
      >
        {{ targets.length ? `Choose where to move ${lib.chosen.value.count === 1 ? "it" : "them"}` : "Add a connection to move them to" }}
      </PButton>
    </template>
  </PScreen>
</template>

<style scoped>
.card { display: flex; flex-direction: column; gap: 12px; padding: 18px; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.card b { font-size: 19px; }
.card p { margin: 0; font-size: 16px; line-height: 1.4; color: var(--text-quiet); }
.line { margin: -6px 2px 12px; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.bad { color: var(--bad); font-weight: 700; }
.bar { display: flex; align-items: center; gap: 8px; margin-bottom: 14px; }
/* The chips scroll; the list/grid switch stays in reach beside them. */
.chips { flex: 1; min-width: 0; display: flex; align-items: center; gap: 8px; overflow-x: auto; scrollbar-width: none; }
.chips > * { flex: none; }
.chip { height: 44px; padding: 0 14px; font: inherit; font-size: 15px; font-weight: 700; white-space: nowrap; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 99px; }
.chip.on { color: var(--panel); background: var(--edge); }
.seg { flex: none; display: flex; margin-left: auto; overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.seg button { width: 44px; height: 40px; display: grid; place-items: center; font: inherit; font-size: 15px; font-weight: 700; color: var(--text); background: none; border: 0; border-right: 2px solid var(--edge); }
.seg button:last-child { border-right: 0; }
.seg button.on { color: var(--panel); background: var(--edge); }
.seg.wide { margin: 0; }
.seg.wide button { flex: 1; width: auto; min-height: 48px; }
.group { margin-bottom: 12px; overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.year { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 52px; padding: 10px 14px; font: inherit; font-size: 15px; text-align: left; color: var(--text); background: var(--rail); border: 0; }
.grow { flex: 1; }
.num { font-weight: 800; }
.item { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 64px; padding: 9px 14px; font: inherit; text-align: left; color: var(--text); background: var(--panel); border: 0; border-top: 2px solid var(--edge); }
.text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.text b { font-size: 16px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.quiet { font-size: 14px; color: var(--text-quiet); }
.check { width: 30px; height: 30px; flex: none; display: grid; place-items: center; font-weight: 900; color: transparent; background: var(--panel); border: 2px solid var(--edge); border-radius: 6px; }
.check.all, .check.some { color: #ffffff; background: var(--tint-drain); }
.grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 6px; padding: 6px; border-top: 2px solid var(--edge); }
.grid .item { position: relative; flex-direction: column; gap: 6px; min-height: 0; padding: 10px 4px; text-align: center; border: 2px solid var(--edge); border-radius: 8px; }
.grid .text b { font-size: 12px; }
.grid .quiet { display: none; }
.grid .check { position: absolute; top: 6px; left: 6px; width: 26px; height: 26px; }
.more { width: 100%; min-height: 48px; font: inherit; font-size: 15px; font-weight: 800; color: var(--text); background: var(--panel); border: 0; border-top: 2px solid var(--edge); }
.label { font-size: 15px; font-weight: 800; }
.says { margin: 0 2px; font-size: 14px; line-height: 1.4; color: var(--text-quiet); }
</style>
