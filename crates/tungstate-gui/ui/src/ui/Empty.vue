<!-- Nothing here yet, and what to do about it. -->
<script setup lang="ts">
defineProps<{ line: string }>();
</script>

<template>
  <div class="nothing">
    <p class="line">{{ line }}</p>
    <slot />
  </div>
</template>

<style scoped>
.nothing { padding: var(--s6) 0; }
.line { font-size: var(--body); color: var(--text-quiet); margin: 0 0 var(--s3); }
</style>
