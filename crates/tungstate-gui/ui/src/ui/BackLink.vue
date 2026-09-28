<!-- The way back, in the same place on every page: small, over the
     heading, where apps and browsers put "back". -->
<script setup lang="ts">
const emit = defineEmits<{ back: [] }>();
</script>

<template>
  <button class="bl" @click="emit('back')"><span aria-hidden="true">‹</span> <slot /></button>
</template>

<style scoped>
.bl {
  align-self: flex-start;
  font: inherit;
  font-size: var(--small);
  color: var(--text-quiet);
  background: none;
  border: none;
  padding: 0;
  margin-bottom: calc(var(--s2) * -1);
  cursor: pointer;
}
.bl:hover { color: var(--text); text-decoration: underline; text-underline-offset: 3px; }
</style>
