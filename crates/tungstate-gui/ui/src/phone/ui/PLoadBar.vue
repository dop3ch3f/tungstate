<!-- Something is on its way with no telling how long: a bar outlined like a
     transfer's, with a block marching across in steps rather than gliding, to
     match the pixel art. Colours come from --panel, --edge and --tint-drain,
     so a dark surface such as the viewer can set its own. -->
<script setup lang="ts">
defineProps<{ label: string }>();
</script>

<template>
  <span class="loadbar" role="progressbar" :aria-label="label"><i></i></span>
</template>

<style scoped>
.loadbar {
  position: relative; display: block; width: 100%; height: 12px; box-sizing: border-box; overflow: hidden;
  background: var(--panel); border: 2px solid var(--edge); border-radius: 3px;
}
/* A third of the bar wide; 333% of its own width carries it clean off the
   far end, since translate percentages are of the block, not the bar. */
.loadbar i { position: absolute; top: 0; bottom: 0; left: 0; width: 30%; background: var(--tint-drain); animation: march 1.2s steps(10) infinite; }
@keyframes march {
  from { transform: translateX(-100%); }
  to { transform: translateX(333%); }
}
@media (prefers-reduced-motion: reduce) {
  .loadbar i { width: 100%; opacity: 0.4; animation: none; }
}
</style>
