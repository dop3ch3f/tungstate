<!-- The Files tab: window tabs along the top, as ES File Explorer has them,
     then Files home or the current tab's folder under them. Each storage and
     connection opens in a tab of its own; copy in one, paste in another. -->
<script setup lang="ts">
import { nextTick, ref, watch } from "vue";
import FilesHome from "../files/FilesHome.vue";
import FileWindow from "../files/FileWindow.vue";
import { CATEGORY_LABEL } from "../lib/look";
import { nameOf, topOf } from "../lib/paths";
import { useStorage } from "../state/useStorage";
import { useWindows, type Place } from "../state/useWindows";

const w = useWindows();
const store = useStorage();

/** What a tab says: the place's name at its top, else the folder's. */
function label(p: Place): string {
  if (p.kind === "home") return "Files";
  if (p.kind === "category") return CATEGORY_LABEL[p.category];
  const top = topOf(p.path, store.tops.value);
  const atTop = top && p.path.replace(/\/+$/, "") === top.path.replace(/\/+$/, "");
  return atTop ? top.label : nameOf(p.path) || top?.label || p.path;
}

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
      <button class="pick" role="tab" :aria-selected="i === w.active.value" @click="w.switchTo(i)">{{ label(x.place) }}</button>
      <button v-if="i === w.active.value && w.wins.value.length > 1" class="x" :aria-label="`Close ${label(x.place)}`" @click="w.close(i)">✕</button>
    </div>
    <button class="add" aria-label="New tab" :disabled="w.wins.value.length >= w.max" @click="w.newTab()">+</button>
  </div>
  <FilesHome v-if="w.win.value.place.kind === 'home'" />
  <FileWindow v-else />
</template>

<style scoped>
.tabs { flex: none; display: flex; align-items: center; gap: 6px; padding: 4px 12px 8px; overflow-x: auto; white-space: nowrap; scrollbar-width: none; }
.tab { flex: none; display: flex; align-items: center; max-width: 168px; height: 40px; background: var(--rail); border: 2px solid var(--edge); border-radius: 8px; }
/* The current tab, the way the current bottom tab is shown. */
.tab:has(> [aria-selected="true"]) { background: var(--panel); box-shadow: 3px 3px 0 var(--edge); }
.pick { min-width: 0; height: 100%; padding: 0 12px; overflow: hidden; text-overflow: ellipsis; font: inherit; font-size: 15px; font-weight: 700; color: var(--text); background: none; border: 0; }
.pick[aria-selected="true"] { font-weight: 800; padding-right: 4px; }
.x { flex: none; width: 34px; height: 100%; font: inherit; font-size: 13px; font-weight: 900; color: var(--text); background: none; border: 0; }
.add { flex: none; width: 40px; height: 40px; font: inherit; font-size: 22px; font-weight: 800; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.add:disabled { opacity: 0.4; }
</style>
