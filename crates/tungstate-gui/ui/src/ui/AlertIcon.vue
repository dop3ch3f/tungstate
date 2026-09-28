<!-- The warning triangle and the failure circle, drawn at text size. Shared by
     Notice and by lists that put several alerts in one box. The shape takes
     `--alert-icon` and the mark knocked out of it `--alert-knock`. -->
<script setup lang="ts">
defineProps<{ tone: "hold" | "bad" }>();
</script>

<template>
  <svg v-if="tone === 'hold'" class="ai" viewBox="0 0 16 16" aria-hidden="true">
    <path class="ai-shape" d="M8 1.2 15.2 14H.8Z" />
    <path class="ai-mark" d="M7.1 5.6h1.8l-.3 4.6H7.4Zm.9 5.6a1 1 0 1 1 0 2 1 1 0 0 1 0-2Z" />
  </svg>
  <svg v-else class="ai" viewBox="0 0 16 16" aria-hidden="true">
    <circle class="ai-shape" cx="8" cy="8" r="7.2" />
    <path class="ai-mark" d="M7.1 3.6h1.8l-.3 5.4H7.4Zm.9 6.6a1.05 1.05 0 1 1 0 2.1 1.05 1.05 0 0 1 0-2.1Z" />
  </svg>
</template>

<style scoped>
/* Centred on the first line of text beside it: that line is 1.55em tall. */
.ai { flex: none; width: 16px; height: 16px; margin-top: calc((1.55em - 16px) / 2); }
.ai-shape { fill: var(--alert-icon); }
.ai-mark { fill: var(--alert-knock); }
</style>
