<!-- The frame, and nothing else.
     A rail with two destinations and a way back, the view, and the one dialog
     host. No feature state lives here: the 543-line version this replaces held
     both halves' state and every modal, which is why nothing in it could be
     moved without moving all of it. -->
<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { useNav } from "./nav";
import { attachTransferStream, detachTransferStream, useTransfer } from "./state/useTransfer";
import { attachDupeStream, detachDupeStream } from "./state/useDupes";
import { attachWatchStream, detachWatchStream, useWatch } from "./state/useWatch";
import Home from "./screens/Home.vue";
import FolderHalf from "./screens/folder/FolderHalf.vue";
import DrainHalf from "./screens/drain/DrainHalf.vue";
import DupesHalf from "./screens/dupes/DupesHalf.vue";
import HistoryView from "./screens/HistoryView.vue";
import SettingsView from "./screens/SettingsView.vue";
import DialogHost from "./ui/DialogHost.vue";
import Mark from "./ui/Mark.vue";
import Tile from "./ui/Tile.vue";
import Window from "./ui/Window.vue";

const nav = useNav();
const t = useTransfer();
/** Whether the open section's window fills the stage. Kept across sections. */
const max = ref(false);

onMounted(() => {
  // First, before anything that can throw. They were once registered last,
  // after four awaits, and when `listen` itself was refused the whole of
  // `onMounted` stopped there: transfers ran and the ledger stayed empty for
  // ever with nothing on screen to say why.
  void attachTransferStream();
  void attachDupeStream();
  void attachWatchStream();
  // What the watcher did before this screen existed: the loop starts when the
  // window does, so by the time anybody looks there may already be a record.
  void useWatch().load();
});
onUnmounted(() => {
  detachTransferStream();
  detachDupeStream();
  detachWatchStream();
});

const WHERE = {
  home: "Home",
  folder: "Organize",
  drain: "Transfer",
  dupes: "Duplicates",
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
      <button class="dest" :class="{ here: nav.view.value === 'settings' }" @click="nav.go('settings')">
        <Tile of="settings" :size="22" />
        Settings
      </button>
    </nav>

    <main class="stage">
      <Home v-if="nav.view.value === 'home'" />
      <Window
        v-else
        :of="nav.view.value"
        :title="nav.view.value === 'settings' ? 'Settings' : WHERE[nav.view.value]"
        :max="max"
        @minimize="nav.go('home')"
        @close="nav.go('home')"
        @maximize="max = !max"
      >
        <FolderHalf v-if="nav.view.value === 'folder'" />
        <DrainHalf v-else-if="nav.view.value === 'drain'" />
        <DupesHalf v-else-if="nav.view.value === 'dupes'" />
        <HistoryView v-else-if="nav.view.value === 'history'" />
        <SettingsView v-else />
      </Window>
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
  border-right: var(--bw) solid var(--edge);
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
/* Retro: the section you are in is a raised, outlined button. The outline is
   an inset shadow so the row does not grow by two borders when chosen. */
:global([data-theme="retro"] .dest.here) {
  background: var(--panel);
  color: var(--text);
  font-weight: 700;
  box-shadow: inset 0 0 0 var(--bw) var(--edge), var(--lift);
}
.gap { flex: 1; }
.live { color: var(--accent); font-size: 9px; padding: var(--s2); }

.stage { flex: 1; position: relative; min-width: 0; }
</style>
