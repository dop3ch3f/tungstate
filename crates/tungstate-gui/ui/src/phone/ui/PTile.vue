<!-- A pixel mark on a colour square, the desktop's section tile at touch size. -->
<script setup lang="ts">
import Pixels from "../../ui/Pixels.vue";
import type { Art } from "../../lib/pixels";

withDefaults(defineProps<{ art: Art; tint: string; light?: boolean; size?: number }>(), { light: false, size: 34 });
</script>

<template>
  <span class="tile" :class="{ light: light }" :style="{ background: tint, width: `${size}px`, height: `${size}px` }">
    <Pixels :of="art" :size="Math.round(size * 0.6)" />
  </span>
</template>

<style scoped>
/* Pixels draws in currentColor. Ink on a pale kind colour, white on a dark
   section colour, as on the desktop's tiles. */
.tile { display: grid; place-items: center; flex: none; color: #151515; border: 2px solid var(--edge); border-radius: 6px; }
.light { color: #ffffff; }
</style>
