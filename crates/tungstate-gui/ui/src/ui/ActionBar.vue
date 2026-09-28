<!-- The bottom of a page or a sheet, built one way everywhere: what to know
     on the left, the choices on the right with the main one last. Pinned, it
     stays in reach however long what is above it, and covers what scrolls
     under it rather than letting it show through. What a press just said
     (a check's answer, an error) goes in `above`, so it is never scrolled
     out of sight behind the bar. -->
<script setup lang="ts">
withDefaults(defineProps<{ pinned?: boolean }>(), { pinned: false });
</script>

<template>
  <footer class="ab" :class="{ 'ab-pinned': pinned }">
    <div class="ab-above" v-if="$slots.above"><slot name="above" /></div>
    <div class="ab-row">
      <div class="ab-say"><slot name="say" /></div>
      <div class="ab-do"><slot /></div>
    </div>
  </footer>
</template>

<style scoped>
.ab {
  display: flex;
  flex-direction: column;
  gap: var(--s3);
  padding: var(--s3) 0 var(--ab-pad);
  border-top: 1px solid var(--rule);
}
.ab-row { display: flex; align-items: center; gap: var(--s3); }
.ab-above { display: flex; flex-direction: column; gap: var(--s2); }
.ab-say { flex: 1; min-width: 0; display: flex; align-items: center; gap: var(--s3); flex-wrap: wrap; font-size: var(--small); color: var(--text-quiet); }
.ab-do { flex: none; display: flex; align-items: center; gap: var(--s2); }
/* A sheet sets --ab-bg and --ab-bottom so the bar sits on its own paper and
   hugs its bottom edge; a page uses the window's. */
.ab-pinned { position: sticky; bottom: var(--ab-bottom); z-index: 1; background: var(--ab-bg); }
</style>
