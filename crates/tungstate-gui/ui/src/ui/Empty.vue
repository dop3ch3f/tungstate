<!-- Nothing here yet, and what to do about it. -->
<script setup lang="ts">
import Pixels from "./Pixels.vue";
import type { Art } from "../lib/pixels";

defineProps<{ line: string; art?: Art }>();
</script>

<template>
  <div class="nothing">
    <Pixels v-if="art" class="scene" :of="art" :size="72" />
    <p class="line">{{ line }}</p>
    <slot />
  </div>
</template>

<style scoped>
.nothing { padding: var(--s6) 0; display: flex; flex-direction: column; align-items: center; text-align: center; gap: var(--s4); }
/* The art takes the colour of the section it is standing in. */
.scene { color: var(--control); opacity: 0.85; }
.line { font-size: var(--body); color: var(--text-quiet); margin: 0; max-width: 52ch; line-height: 1.5; }
</style>
