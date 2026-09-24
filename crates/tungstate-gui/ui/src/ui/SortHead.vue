<!-- A column header that sorts its table. The arrow shows only on the column
     in charge, so a row of headers reads as labels until one is used. -->
<script setup lang="ts" generic="T">
import type { Table } from "../lib/table";

const props = defineProps<{ table: Table<T>; column: string; numeric?: boolean }>();
</script>

<template>
  <button
    type="button"
    class="sh"
    :class="{ 'sh-num': props.numeric }"
    :aria-sort="props.table.sort.value.key !== props.column ? 'none' : props.table.sort.value.dir === 1 ? 'ascending' : 'descending'"
    @click="props.table.by(props.column, props.numeric)"
  >
    <slot />
    <span class="sh-arrow" v-if="props.table.sort.value.key === props.column">{{ props.table.sort.value.dir === 1 ? "▲" : "▼" }}</span>
  </button>
</template>

<style scoped>
.sh { font: inherit; background: none; border: none; color: inherit; padding: 0; cursor: pointer; text-align: left; display: inline-flex; gap: 4px; align-items: baseline; }
.sh-num { justify-content: flex-end; width: 100%; }
.sh:hover { color: var(--text); }
.sh-arrow { font-size: 0.8em; }
</style>
