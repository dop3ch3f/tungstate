<!-- A wait with a name.
     Tidying emits no events (docs/SEAM.md, "what is deliberately not in the
     seam yet"), so a long one reports nothing until it returns. This says so
     rather than drawing a bar that cannot fill, and offers no cancel, because
     there is nothing to cancel. -->
<script setup lang="ts">
defineProps<{ what: string; note?: string }>();
</script>

<template>
  <div class="working" aria-busy="true">
    <span class="pulse" aria-hidden="true"></span>
    <span class="what">{{ what }}</span>
    <span class="aside" v-if="note">{{ note }}</span>
  </div>
</template>

<style scoped>
.working { display: flex; align-items: center; gap: var(--s2); padding: var(--s3) 0; }
[aria-busy="true"] { cursor: progress; }
.pulse {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--accent);
  animation: breathe 1.6s var(--ease) infinite;
}
.what { font-size: var(--body); }
.aside { font-size: var(--small); color: var(--text-faint); }
@keyframes breathe { 0%, 100% { opacity: 0.25 } 50% { opacity: 1 } }
@media (prefers-reduced-motion: reduce) { .pulse { animation: none; opacity: 1 } }
</style>
