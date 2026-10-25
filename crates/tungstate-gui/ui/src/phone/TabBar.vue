<!-- Five tabs. The current one is white, bold and outlined with the hard
     shadow, the desktop's rule for the chosen item. The frame around it
     adds the home-bar inset below. -->
<script setup lang="ts">
import Pixels from "../ui/Pixels.vue";
import type { Art } from "../lib/pixels";
import { usePhoneNav, type Tab } from "./nav";

const nav = usePhoneNav();

const TABS: { id: Tab; art: Art; label: string }[] = [
  { id: "home", art: "home", label: "Home" },
  { id: "free", art: "drain", label: "Free up" },
  { id: "dupes", art: "dupes", label: "Duplicates" },
  { id: "organise", art: "folder", label: "Organise" },
  { id: "connections", art: "connections", label: "Connections" },
];
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
      <Pixels :of="t.art" :size="26" />
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
  height: 58px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 3px;
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
</style>
