<!-- One labelled input in a form: the label above, a note under it when the
     field needs explaining. The input itself is the slot. -->
<script setup lang="ts">
defineProps<{ label: string; note?: string }>();
</script>

<template>
  <label class="fld">
    <span class="fld-l">{{ label }}</span>
    <slot />
    <span class="fld-n" v-if="note">{{ note }}</span>
  </label>
</template>

<style scoped>
.fld { display: flex; flex-direction: column; gap: 5px; min-width: 0; }
.fld-l { font-size: var(--small); font-weight: 600; }
.fld-n { font-size: var(--fine); color: var(--text-faint); line-height: 1.45; }
.fld :deep(input), .fld :deep(select) { width: 100%; }
</style>
