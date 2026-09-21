<!-- The periodic tile, at in-product size: the name and corner marks dropped,
     because at 20px they are noise rather than detail. docs/brand/README.md -->
<script setup lang="ts">
withDefaults(defineProps<{ size?: number }>(), { size: 20 });
</script>

<template>
  <svg class="mark" :width="size" :height="size" viewBox="0 0 32 32" aria-label="Tungstate">
    <rect x="1" y="1" width="30" height="30" rx="5" fill="none" stroke="currentColor" stroke-width="1.5" />
    <text x="16" y="21" text-anchor="middle" class="wo">WO</text>
  </svg>
</template>

<style scoped>
.mark { display: block; flex: none; }
.wo { font-family: var(--font); font-size: 12px; font-weight: 600; fill: currentColor; }
</style>
