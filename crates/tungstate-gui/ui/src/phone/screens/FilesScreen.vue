<!-- The Files tab: Files home, or the window open on a folder, with the
     window switcher above the tabs once a second window is open. -->
<script setup lang="ts">
import FilesHome from "../files/FilesHome.vue";
import FileWindow from "../files/FileWindow.vue";
import { CATEGORY_LABEL } from "../lib/look";
import { connectionOf, nameOf } from "../lib/paths";
import { useWindows, type Place } from "../state/useWindows";

const w = useWindows();

/** What a window's chip says: the folder, or the connection at its top. */
function label(p: Place): string {
  if (p.kind === "home") return "Files";
  if (p.kind === "category") return CATEGORY_LABEL[p.category];
  const conn = connectionOf(p.path);
  const name = nameOf(p.path);
  return conn && name ? `${conn} › ${name}` : conn ?? name;
}
</script>

<template>
  <FilesHome v-if="w.win.value.place.kind === 'home'" />
  <FileWindow v-else />
  <div v-if="w.wins.value.length > 1 && !w.selecting.value" class="winbar">
    <button v-for="(x, i) in w.wins.value" :key="i" class="window-chip" :aria-current="i === w.active.value ? 'true' : undefined" @click="w.switchTo(i)">
      <span class="n">{{ i + 1 }}</span><span class="name">{{ label(x.place) }}</span>
    </button>
  </div>
</template>

<style scoped>
.winbar { flex: none; display: flex; gap: 8px; padding: 8px 12px; background: var(--rail); border-top: 2px solid var(--edge); }
.window-chip { flex: 1; min-width: 0; height: 44px; display: flex; align-items: center; gap: 8px; padding: 0 10px; font: inherit; font-size: 15px; font-weight: 700; color: var(--text); background: var(--rail); border: 2px solid var(--edge); border-radius: 8px; }
/* The current window, the way the current tab is shown. */
.window-chip[aria-current] { font-weight: 800; background: var(--panel); box-shadow: var(--lift); }
.n { width: 22px; height: 22px; flex: none; display: grid; place-items: center; font-size: 12px; font-weight: 900; background: var(--panel); border: 2px solid var(--edge); border-radius: 4px; }
.name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
