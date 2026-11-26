<!-- The Files tab: window tabs along the top, as ES File Explorer has them,
     then Files home or the current tab's folder under them. Home is first and
     never closes; connections and cards open in tabs of their own. Each tab
     carries its place's mark and, inside a folder, the place's name, so two
     "Photos" on different places are never confused. -->
<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import FilesHome from "../files/FilesHome.vue";
import FileWindow from "../files/FileWindow.vue";
import { useConnections } from "../../state/useConnections";
import { CATEGORIES, HOME_LOOK, folderLook, type Look } from "../lib/look";
import { nameOf, topOf } from "../lib/paths";
import { useStorage } from "../state/useStorage";
import { HOME, useWindows, type Place } from "../state/useWindows";
import PTile from "../ui/PTile.vue";

const w = useWindows();
const store = useStorage();
const conns = useConnections();
const schemeOf = (name: string) => conns.all.value.find((c) => c.name === name)?.scheme;

interface Face extends Look {
  name: string;
  /** The place a folder is on, when the name is the folder's own. */
  on: string | null;
}

function face(p: Place, i: number): Face {
  if (p.kind === "home") return { ...HOME_LOOK, name: i === HOME ? "Home" : "New tab", on: null };
  if (p.kind === "category") {
    const c = CATEGORIES.find((x) => x.id === p.category)!;
    return { art: c.art, tint: c.tint, light: false, name: c.label, on: "This phone" };
  }
  const look = folderLook(p.path, store.volumes.value, schemeOf);
  const top = topOf(p.path, store.tops.value);
  const atTop = top && p.path.replace(/\/+$/, "") === top.path.replace(/\/+$/, "");
  return { ...look, name: atTop ? top.label : nameOf(p.path) || top?.label || p.path, on: atTop ? null : top?.label ?? null };
}

const faces = computed(() => w.wins.value.map((x, i) => face(x.place, i)));

// A new or chosen tab past the edge of the strip is scrolled into view.
const strip = ref<HTMLElement | null>(null);
watch([w.active, () => w.wins.value.length], () => void nextTick(() => {
  strip.value?.querySelector('[aria-selected="true"]')?.scrollIntoView({ inline: "nearest", block: "nearest" });
}));
</script>

<template>
  <!-- Stays while ticking, as in ES: taking it away moved every row up
       under the finger mid-press. -->
  <div ref="strip" class="tabs" role="tablist" aria-label="Windows">
    <div v-for="(x, i) in w.wins.value" :key="i" class="tab">
      <button class="pick" role="tab" :aria-selected="i === w.active.value" @click="w.switchTo(i)">
        <PTile :art="faces[i]!.art" :tint="faces[i]!.tint" :light="faces[i]!.light" :size="26" />
        <span class="words"><b>{{ faces[i]!.name }}</b><small v-if="faces[i]!.on">{{ faces[i]!.on }}</small></span>
      </button>
      <button v-if="i === w.active.value && i !== HOME" class="x" :aria-label="`Close ${faces[i]!.name}`" @click="w.close(i)">✕</button>
    </div>
    <button class="add" aria-label="New tab" :disabled="w.wins.value.length >= w.max" @click="w.newTab()">+</button>
  </div>
  <FilesHome v-if="w.win.value.place.kind === 'home'" />
  <FileWindow v-else />
</template>

<style scoped>
.tabs { flex: none; display: flex; align-items: center; gap: 6px; padding: 4px 12px 8px; overflow-x: auto; white-space: nowrap; scrollbar-width: none; }
.tab { flex: none; display: flex; align-items: center; max-width: 188px; height: 48px; background: var(--rail); border: 2px solid var(--edge); border-radius: 8px; }
/* The current tab, the way the current bottom tab is shown. */
.tab:has(> [aria-selected="true"]) { background: var(--panel); box-shadow: 3px 3px 0 var(--edge); }
.pick { display: flex; align-items: center; gap: 8px; min-width: 0; height: 100%; padding: 0 12px 0 8px; font: inherit; text-align: left; color: var(--text); background: none; border: 0; }
.pick[aria-selected="true"] { padding-right: 4px; }
.pick :deep(.tile) { border-width: 1.5px; border-radius: 5px; }
.words { display: flex; flex-direction: column; min-width: 0; line-height: 1.15; }
.words b, .words small { overflow: hidden; text-overflow: ellipsis; }
.words b { font-size: 15px; font-weight: 700; }
.pick[aria-selected="true"] b { font-weight: 800; }
.words small { font-size: 12px; font-weight: 600; color: var(--text-quiet); }
.x { flex: none; width: 34px; height: 100%; font: inherit; font-size: 13px; font-weight: 900; color: var(--text); background: none; border: 0; }
.add { flex: none; width: 48px; height: 48px; font: inherit; font-size: 22px; font-weight: 800; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.add:disabled { opacity: 0.4; }
</style>
