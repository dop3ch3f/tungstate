<!-- One of a few choices, drawn as a joined strip that shows which is on.
     Tabs choose what you see; this chooses how something behaves. -->
<script setup lang="ts" generic="T extends string">
const props = defineProps<{ options: readonly { id: T; label: string }[]; modelValue: T | null; label: string }>();
const emit = defineEmits<{ "update:modelValue": [value: T] }>();
</script>

<template>
  <span class="sg" role="radiogroup" :aria-label="props.label">
    <button
      v-for="o in props.options"
      :key="o.id"
      role="radio"
      :aria-checked="props.modelValue === o.id"
      class="sg-b"
      :class="{ 'sg-on': props.modelValue === o.id }"
      @click="emit('update:modelValue', o.id)"
    >{{ o.label }}</button>
  </span>
</template>

<style scoped>
.sg { display: inline-flex; gap: 2px; padding: 2px; background: var(--rail); border-radius: var(--radius); }
.sg-b {
  font: inherit;
  font-size: var(--small);
  color: var(--text-quiet);
  background: none;
  border: none;
  border-radius: var(--radius);
  padding: 3px var(--s3);
  cursor: pointer;
}
.sg-b:hover { color: var(--text); }
.sg-on { color: var(--text); background: var(--surface-raised); font-weight: 600; }
/* Retro: one outlined strip, as the tabs are, the choice filled like the rail's. */
:global([data-theme="retro"] .sg) { gap: 0; padding: 0; background: var(--surface-raised); border: var(--bw) solid var(--edge); overflow: hidden; }
:global([data-theme="retro"] .sg-b) { border-radius: 0; border-right: var(--bw) solid var(--edge); color: var(--text); }
:global([data-theme="retro"] .sg-b:last-child) { border-right: none; }
:global([data-theme="retro"] .sg-on) { background: var(--chosen); color: var(--chosen-ink); font-weight: 700; }
</style>
