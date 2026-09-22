<!-- The dialog shell: one veil, one card, focus kept inside it, Escape closes.
     Every question in the window renders through this. -->
<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, nextTick } from "vue";

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
  focusable()[0]?.focus();
  window.addEventListener("keydown", keydown);
});
onBeforeUnmount(() => {
  window.removeEventListener("keydown", keydown);
  restoreTo?.focus();
});
</script>

<template>
  <div class="scrim" @click.self="emit('dismiss')">
    <div ref="panel" class="panel" role="dialog" aria-modal="true">
      <slot />
    </div>
  </div>
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
  border: 1px solid var(--edge);
  border-radius: var(--radius-xl);
  padding: var(--s5) var(--s5) var(--s4);
  box-shadow: var(--lift-panel), var(--drop);
}
@keyframes fade { from { opacity: 0 } to { opacity: 1 } }
@media (prefers-reduced-motion: reduce) { .scrim { animation: none } }
</style>
