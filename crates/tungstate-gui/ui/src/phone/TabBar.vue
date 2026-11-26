<!-- Five tabs, Files first (docs/mobile/approved-files/). Each mark sits on
     its section's colour square, as the desktop's sidebar draws them. The
     current one is raised, bold and outlined with the hard shadow, the
     desktop's rule for the chosen item. The frame around it adds the
     home-bar inset below. -->
<script setup lang="ts">
import type { Art } from "../lib/pixels";
import PTile from "./ui/PTile.vue";
import { usePhoneNav, type Tab } from "./nav";
import { isAndroid } from "./platform";

const nav = usePhoneNav();

// The desktop's section colours: home for Files, moving files for Free up.
const ALL: { id: Tab; art: Art; label: string; tint: string; light: boolean }[] = [
  { id: "files", art: "folder", label: "Files", tint: "var(--tint-home)", light: false },
  { id: "free", art: "drain", label: "Free up", tint: "var(--tint-drain)", light: true },
  { id: "dupes", art: "dupes", label: "Duplicates", tint: "var(--tint-dupes)", light: true },
  { id: "organise", art: "organise", label: "Organise", tint: "var(--tint-folder)", light: true },
  { id: "connections", art: "connections", label: "Connections", tint: "var(--tint-connections)", light: true },
];
// Duplicates and Organise walk the phone's folders, which an iPhone app
// cannot see; they show where they work, on Android.
const TABS = isAndroid() ? ALL : ALL.filter((t) => t.id !== "dupes" && t.id !== "organise");
</script>

<template>
  <nav class="tabs">
    <button
      v-for="t in TABS"
      :key="t.id"
      class="tab"
      :aria-current="nav.tab.value === t.id ? 'page' : undefined"
      @click="nav.go(t.id)"
    >
      <PTile :art="t.art" :tint="t.tint" :light="t.light" :size="30" />
      <span>{{ t.label }}</span>
    </button>
  </nav>
</template>

<style scoped>
.tabs {
  flex: none;
  display: flex;
  gap: 2px;
  padding: 8px 6px 6px;
  border-top: 2px solid var(--edge);
  background: var(--rail);
}
.tab {
  /* Sized by their labels, so the long one gets the room it needs. */
  flex: 1 1 auto;
  min-width: 0;
  padding: 0 4px;
  height: 62px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  font: inherit;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: -0.3px;
  white-space: nowrap;
  color: var(--text);
  background: none;
  border: 2px solid transparent;
  border-radius: 8px;
}
.tab[aria-current="page"] { background: var(--panel); border-color: var(--edge); box-shadow: var(--lift); font-weight: 800; }
.tab :deep(.tile) { border-width: 1.5px; }
</style>
