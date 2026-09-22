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

// Three strokes each on a 16pt grid, drawn in the label's own colour.
const WHERE = {
  folder: { label: "Tidy a folder", icon: "M1.5 4.5v8h13v-7H7.5L6 4H1.5zM4 9h8" },
  drain: { label: "Move to another machine", icon: "M3 2.5h10v4H3zM3 9.5h10v4H3zM8 6.5v3M6.5 8L8 9.5 9.5 8" },
  history: { label: "What has happened", icon: "M8 2a6 6 0 1 0 0 12A6 6 0 0 0 8 2zM8 4.5V8l2.5 1.5" },
} as const;
</script>

<template>
  <div class="frame" :data-half="nav.view.value">
    <nav class="rail">
      <button class="badge" @click="nav.go('home')" title="Home">
        <Mark :size="26" />
      </button>
      <button
        v-for="(d, key) in WHERE"
        :key="key"
        class="dest"
        :class="{ here: nav.view.value === key }"
        @click="nav.go(key)"
      >
        <svg class="ico" viewBox="0 0 16 16" aria-hidden="true"><path :d="d.icon" /></svg>
        {{ d.label }}
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
/* The field is the whole window, rail included, so the rail reads as glass
   laid over the colour rather than a grey column beside it. */
.frame {
  display: flex;
  height: 100%;
  background: var(--grain), var(--field);
  transition: background var(--slow) var(--ease);
}

.rail {
  width: 220px;
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
  padding: var(--s2) var(--s3);
  border-radius: var(--radius);
  cursor: pointer;
}
.dest:hover { color: var(--text); background: var(--surface-hover); }
.dest.here { color: var(--text); background: var(--surface-raised); }
.gap { flex: 1; }
.ico { width: 16px; height: 16px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.3; stroke-linecap: round; stroke-linejoin: round; opacity: 0.85; }
.live { color: var(--accent); font-size: 9px; padding: var(--s2); }

.stage { flex: 1; position: relative; min-width: 0; }
</style>
