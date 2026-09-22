<!-- A pixel-art grid drawn as squares. One path per colour, so an icon is two
     elements however many squares it has, and it stays crisp at any size. -->
<script setup lang="ts">
import { computed } from "vue";
import { ART, type Art } from "../lib/pixels";

const props = withDefaults(defineProps<{ of: Art; size?: number }>(), { size: 16 });

/** Every square of one character, as a single path of 1x1 boxes. */
function squares(mark: string): string {
  const rows = ART[props.of];
  let d = "";
  rows.forEach((row, y) => {
    [...row].forEach((cell, x) => {
      if (cell === mark) d += `M${x} ${y}h1v1h-1z`;
    });
  });
  return d;
}

const side = computed(() => ART[props.of][0].length);
const ink = computed(() => squares("x"));
const second = computed(() => squares("o"));
</script>

<template>
  <svg
    class="px"
    :width="props.size"
    :height="props.size"
    :viewBox="`0 0 ${side} ${side}`"
    aria-hidden="true"
  >
    <path class="px-ink" :d="ink" />
    <path class="px-second" :d="second" />
  </svg>
</template>

<style scoped>
.px { display: block; flex: none; shape-rendering: crispEdges; }
.px-ink { fill: currentColor; }
.px-second { fill: currentColor; opacity: 0.45; }
</style>
