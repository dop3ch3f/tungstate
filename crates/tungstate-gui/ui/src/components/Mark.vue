<script setup lang="ts">
// The compact mark: the tile and WO only. At the sizes the interface uses it,
// the charge, subscript and name would be noise rather than detail, so the
// full mark in docs/brand keeps those and this one does not.
withDefaults(defineProps<{ size?: number }>(), { size: 22 });
</script>

<template>
  <svg :style="{ width: `${size}px`, height: `${size}px` }"
       viewBox="104 104 304 304" aria-hidden="true" class="mark">
    <rect x="116.00" y="116.00" width="280.00" height="280.00" rx="26.00" fill="none" stroke="#7fd4ff" stroke-width="16.00"/><text x="256.00" y="302.00" font-family="ui-sans-serif,-apple-system,Helvetica Neue,Helvetica,Arial" font-size="150.00" font-weight="600" fill="#7fd4ff" text-anchor="middle" letter-spacing="-5.00">WO</text>
  </svg>
</template>

<style scoped>
/* Size comes from an inline style, not the width and height attributes:
   those are presentation attributes and any stylesheet rule outranks them. */
.mark { display: block; flex: none; }
</style>
