<!-- The dialog shell: one veil, one card, focus kept inside it, Escape closes.
     Every question in the window renders through this. -->
<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, nextTick } from "vue";

import TitleBar from "./TitleBar.vue";
import type { Section } from "../lib/icons";

const props = defineProps<{ wide?: boolean; title?: string; of?: Section }>();
const emit = defineEmits<{ dismiss: [] }>();
const card = ref<HTMLElement | null>(null);
let restoreTo: HTMLElement | null = null;

const focusable = () =>
  Array.from(
    card.value?.querySelectorAll<HTMLElement>(
      'button:not(:disabled), [href], input:not(:disabled), select, textarea, [tabindex]:not([tabindex="-1"])',
    ) ?? [],
  );

/** Tab must not walk out of a modal dialog and start driving the window
   behind it, which is what all twelve of the hand-rolled ones allowed. */
function keydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    e.preventDefault();
    emit("dismiss");
    return;
  }
  if (e.key !== "Tab") return;
  const items = focusable();
  if (items.length === 0) return;
  const first = items[0];
  const last = items[items.length - 1];
  if (e.shiftKey && document.activeElement === first) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && document.activeElement === last) {
    e.preventDefault();
    first.focus();
  }
}

onMounted(async () => {
  restoreTo = document.activeElement as HTMLElement | null;
  await nextTick();
  // Not `focusable()[0]`: that is the title bar's minimise button, and a
  // focus ring on window furniture is not what a question should open on.
  (card.value?.querySelector<HTMLElement>(".inside button, .inside input") ?? focusable()[0])?.focus();
  window.addEventListener("keydown", keydown);
});
onBeforeUnmount(() => {
  window.removeEventListener("keydown", keydown);
  restoreTo?.focus();
});
</script>

<template>
  <!-- Teleported: a dialog raised from inside a pane was being positioned
       against that pane rather than the window, because a `container-type`
       ancestor is a containing block even for `position: fixed`. -->
  <Teleport to=".frame">
    <div class="scrim" @click.self="emit('dismiss')">
      <div ref="card" class="panel" :class="{ 'panel-wide': props.wide }" role="dialog" aria-modal="true">
        <TitleBar
          v-if="props.title"
          class="cap"
          :of="props.of ?? 'settings'"
          :title="props.title"
          controls="live"
          @minimize="emit('dismiss')"
          @maximize="emit('dismiss')"
          @close="emit('dismiss')"
        />
        <div class="inside"><slot /></div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  display: grid;
  place-items: center;
  background: var(--veil);
  animation: fade var(--slow) var(--ease);
  z-index: 10;
}
.panel {
  width: min(460px, calc(100vw - var(--s6) * 2));
  background: var(--sheet);
  color: var(--text);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius-xl);
  padding: 0;
  overflow: hidden;
  box-shadow: var(--lift-panel), var(--drop);
}
.inside { padding: var(--s5) var(--s5) var(--s4); }
.cap { padding: var(--s4) var(--s5) 0; }
:global([data-theme="retro"] .cap) { padding: 6px 7px 6px var(--s3); }
.panel-wide { width: min(580px, calc(100vw - var(--s6) * 2)); max-height: calc(100vh - var(--s6) * 2); overflow-y: auto; }
@keyframes fade { from { opacity: 0 } to { opacity: 1 } }
@media (prefers-reduced-motion: reduce) { .scrim { animation: none } }
</style>
