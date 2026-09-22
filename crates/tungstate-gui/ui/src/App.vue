<!-- The frame, and nothing else.
     A rail with two destinations and a way back, the view, and the one dialog
     host. No feature state lives here: the 543-line version this replaces held
     both halves' state and every modal, which is why nothing in it could be
     moved without moving all of it. -->
<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import { useNav } from "./nav";
import { attachTransferStream, detachTransferStream, useTransfer } from "./state/useTransfer";
import Home from "./screens/Home.vue";
import FolderHalf from "./screens/folder/FolderHalf.vue";
import DrainHalf from "./screens/drain/DrainHalf.vue";
import HistoryView from "./screens/HistoryView.vue";
import DialogHost from "./ui/DialogHost.vue";
import Mark from "./ui/Mark.vue";
import Tile from "./ui/Tile.vue";

const nav = useNav();
const t = useTransfer();

onMounted(() => {
  // First, before anything that can throw. They were once registered last,
  // after four awaits, and when `listen` itself was refused the whole of
  // `onMounted` stopped there: transfers ran and the ledger stayed empty for
  // ever with nothing on screen to say why.
  void attachTransferStream();
});
onUnmounted(detachTransferStream);

const WHERE = {
  home: "Home",
  folder: "Organize",
  drain: "Transfer",
  history: "History",
} as const;
</script>

<template>
  <div class="frame" :data-half="nav.view.value">
    <nav class="rail">
      <button class="badge" @click="nav.go('home')" title="Home">
        <Mark :size="26" />
      </button>
      <button
        v-for="(label, key) in WHERE"
        :key="key"
        class="dest"
        :class="{ here: nav.view.value === key }"
        @click="nav.go(key)"
      >
        <Tile :of="key" :size="22" />
        {{ label }}
      </button>
      <span class="gap"></span>
      <span class="live" v-if="t.running.value" title="A transfer is running">●</span>
    </nav>

    <main class="stage">
      <Home v-if="nav.view.value === 'home'" />
      <FolderHalf v-else-if="nav.view.value === 'folder'" />
      <DrainHalf v-else-if="nav.view.value === 'drain'" />
      <HistoryView v-else />
    </main>

    <DialogHost />
  </div>
</template>

<style scoped>
.frame { display: flex; height: 100%; background: var(--field); }

.rail {
  width: 172px;
  flex: none;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: var(--s4) var(--s2);
  background: var(--rail);
  border-right: 1px solid var(--edge);
}
.badge {
  background: none;
  border: none;
  color: var(--text-quiet);
  padding: var(--s2);
  margin-bottom: var(--s3);
  cursor: pointer;
  align-self: flex-start;
}
.badge:hover { color: var(--text-quiet); }

.dest {
  display: flex;
  align-items: center;
  gap: var(--s2);
  font: inherit;
  font-size: var(--nav);
  font-weight: 500;
  white-space: nowrap;
  text-align: left;
  background: none;
  border: none;
  color: var(--text-quiet);
  padding: 5px var(--s2);
  border-radius: var(--radius);
  cursor: pointer;
}
.dest:hover { color: var(--text); background: var(--surface-hover); }
.dest.here { color: var(--text); background: var(--surface-hover); }
/* Retro: the section you are in is a raised, outlined button. */
:global([data-theme="retro"] .dest.here) { background: var(--panel); box-shadow: inset 0 0 0 1px var(--edge), var(--lift); }
.gap { flex: 1; }
.live { color: var(--accent); font-size: 9px; padding: var(--s2); }

.stage { flex: 1; position: relative; min-width: 0; }
</style>
