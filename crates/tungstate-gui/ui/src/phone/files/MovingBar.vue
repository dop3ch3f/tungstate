<!-- A move left running with "Keep browsing": one line above the tabs, so
     it is never out of sight. It fills as the move goes, and its arrows step
     while files are moving. Tapping it opens the Moving screen again. -->
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
const live = computed(() => t.current.value?.id === run.value.job);
const n = computed(() => tally(live.value ? t.rows.value : []));
const pct = computed(() => (ended.value ? 100 : n.value.total ? Math.round((n.value.bytes / n.value.total) * 100) : 0));
</script>

<template>
  <button class="moving" @click="$emit('show')">
    <i class="fill" :style="{ width: `${pct}%` }" aria-hidden="true"></i>
    <span v-if="live" class="arrows" aria-hidden="true"><i>›</i><i>›</i><i>›</i></span>
    <span class="grow">{{ ended ? `${run.op === "move" ? "Move" : "Copy"} finished` : `${run.op === "move" ? "Moving" : "Copying"} ${run.what} · ${n.sent} of ${n.count} · ${pct}%` }}</span>
    <span class="chev" aria-hidden="true">›</span>
  </button>
</template>

<style scoped>
.moving { position: relative; overflow: hidden; display: flex; align-items: center; gap: 8px; width: calc(100% - 24px); min-height: 48px; margin: 0 12px 8px; padding: 0 14px; font: inherit; font-size: 15px; font-weight: 800; color: #ffffff; background: var(--tint-drain); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 3px 3px 0 var(--edge); }
/* How far, as a darker band under the words. */
.fill { position: absolute; inset: 0 auto 0 0; background: color-mix(in srgb, #000000 22%, transparent); transition: width 0.3s ease-out; }
.arrows, .grow, .chev { position: relative; }
.arrows { display: flex; font-size: 20px; letter-spacing: -3px; }
.arrows i { font-style: normal; opacity: 0.35; animation: lit 0.9s steps(1) infinite; }
.arrows i:nth-child(2) { animation-delay: 0.3s; }
.arrows i:nth-child(3) { animation-delay: 0.6s; }
@keyframes lit { 0% { opacity: 1; } 33% { opacity: 0.35; } }
@media (prefers-reduced-motion: reduce) { .arrows i { animation: none; opacity: 1; } }
.grow { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: left; }
.chev { font-size: 20px; }
</style>
