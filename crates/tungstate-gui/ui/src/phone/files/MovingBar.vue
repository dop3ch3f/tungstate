<!-- A move left running with "Keep browsing": one line above the tabs, so
     it is never out of sight. Tapping it opens the Moving screen again. -->
<script setup lang="ts">
import { computed } from "vue";
import { useTransfer } from "../../state/useTransfer";
import { tally } from "../lib/progress";
import { useClipboard } from "../state/useClipboard";

defineEmits<{ show: [] }>();
const board = useClipboard();
const t = useTransfer();
const run = computed(() => board.started.value!);
const ended = computed(() => t.finished.value.some((r) => r.job.id === run.value.job));
const n = computed(() => tally(t.current.value?.id === run.value.job ? t.rows.value : []));
const pct = computed(() => (n.value.total ? Math.round((n.value.bytes / n.value.total) * 100) : 0));
</script>

<template>
  <button class="moving" @click="$emit('show')">
    <span class="grow">{{ ended ? `${run.op === "move" ? "Move" : "Copy"} finished` : `${run.op === "move" ? "Moving" : "Copying"} ${run.what} · ${pct}%` }}</span>
    <span class="chev" aria-hidden="true">›</span>
  </button>
</template>

<style scoped>
.moving { display: flex; align-items: center; gap: 8px; width: calc(100% - 24px); min-height: 48px; margin: 0 12px 8px; padding: 0 14px; font: inherit; font-size: 15px; font-weight: 800; color: #ffffff; background: var(--tint-drain); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 3px 3px 0 var(--edge); }
.grow { flex: 1; text-align: left; }
.chev { font-size: 20px; }
</style>
