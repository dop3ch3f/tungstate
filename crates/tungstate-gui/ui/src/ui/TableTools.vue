<!-- The search box and filter chips over a table. Pairs with `lib/table.ts`,
     so every long list in the app narrows the same way. -->
<script setup lang="ts" generic="T">
import { computed } from "vue";
import type { Table } from "../lib/table";

const props = defineProps<{
  table: Table<T>;
  placeholder: string;
  /** How a facet's raw value reads on a chip, when it is not already a word. */
  say?: (facet: string, value: string) => string;
}>();
const word = (facet: string, value: string) => props.say?.(facet, value) ?? value;

// One value filters nothing, and past a dozen the chips are a second list to
// read rather than a shortcut; the search box covers both cases.
const useful = computed(() =>
  props.table.facetValues.value.filter((f) => f.values.length > 1 && f.values.length <= 12),
);
</script>

<template>
  <div class="tt">
    <input
      v-model="props.table.query.value"
      class="tt-in"
      type="search"
      :placeholder="props.placeholder"
      :aria-label="props.placeholder"
    />
    <div v-for="facet in useful" :key="facet.key" class="tt-facet" role="group" :aria-label="facet.label">
      <span class="tt-label">{{ facet.label }}</span>
      <button
        v-for="option in facet.values"
        :key="option.value"
        type="button"
        class="tt-chip"
        :class="{ 'tt-on': props.table.chosen.value[facet.key] === option.value }"
        :aria-pressed="props.table.chosen.value[facet.key] === option.value"
        @click="props.table.pick(facet.key, option.value)"
      >
        {{ word(facet.key, option.value) }} <span class="tt-n">{{ option.count }}</span>
      </button>
    </div>
    <span class="tt-count" v-if="props.table.narrowed.value">
      {{ props.table.shown.value.length }} of {{ props.table.total.value }}
      <button type="button" class="tt-clear" @click="props.table.clear()">Clear</button>
    </span>
  </div>
</template>

<style scoped>
.tt { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2) var(--s4); margin-bottom: var(--s3); }
.tt-in { min-width: 220px; flex: 0 1 280px; }
.tt-facet { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; }
.tt-label { font-size: var(--fine); color: var(--text-faint); margin-right: 2px; }
.tt-chip {
  font: inherit;
  font-size: var(--fine);
  padding: 2px 8px;
  border: 1px solid var(--edge);
  border-radius: 999px;
  background: var(--panel);
  color: var(--text-quiet);
  cursor: pointer;
}
.tt-chip:hover { color: var(--text); background: var(--surface-hover); }
.tt-on { color: var(--text); background: var(--surface-raised); font-weight: 700; }
.tt-n { color: var(--text-faint); font-variant-numeric: tabular-nums; }
.tt-count { font-size: var(--fine); color: var(--text-faint); margin-left: auto; }
.tt-clear { font: inherit; background: none; border: none; padding: 0 0 0 var(--s2); color: var(--text); text-decoration: underline; cursor: pointer; }
/* Retro: chips are the same outlined blocks as every other control. */
:global([data-theme="retro"] .tt-chip) { border-width: var(--bw); border-radius: var(--radius); }
:global([data-theme="retro"] .tt-on) { box-shadow: var(--lift); }
</style>
