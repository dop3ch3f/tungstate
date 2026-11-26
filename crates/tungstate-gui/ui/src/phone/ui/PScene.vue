<!-- The two wide pixel scenes from the approved Free up screens, 24 by 12
     squares: a phone sending to a NAS, and the NAS holding it all. Wide, so
     not drawn by the square icon renderer. With `moving`, the three arrows
     between them light one after another, so a send never looks stuck. -->
<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{ of: "send" | "done"; moving?: boolean }>();

// x: ink, o: the section colour, a/b/c: the three arrows, in the order they light.
const SCENES = {
  send: [
    "........................", ".xxxxx..............xxxx", ".x...x.............xoooo", ".x.o.x.............xxxxx",
    ".x...x...a..b..c...xoooo", ".x...x....a..b..c..xxxxx", ".x...x...a..b..c...xoooo", ".x...x.............xxxxx",
    ".x...x.............xoooo", ".xxxxx.............xxxxx", "..xx...............xx.xx", "........................",
  ],
  done: [
    "........................", "..........xxxx..........", ".........xoooox.........", ".........xxxxxx.........",
    ".........xoooox....x....", ".........xxxxxx...xx....", ".........xoooox..xx.....", ".........xxxxxx.xx......",
    ".........xoooox.x.......", ".........xxxxxx.........", "..........xx.xx.........", "........................",
  ],
};

function squares(mark: string): string {
  let d = "";
  SCENES[props.of].forEach((row, y) => [...row].forEach((c, x) => { if (c === mark) d += `M${x} ${y}h1v1h-1z`; }));
  return d;
}
const ink = computed(() => squares("x"));
const second = computed(() => squares("o"));
const arrows = computed(() => ["a", "b", "c"].map(squares));
</script>

<template>
  <svg class="scene" :class="{ moving: moving }" viewBox="0 0 24 12" aria-hidden="true">
    <path :d="second" fill="var(--tint-drain)" />
    <path :d="ink" fill="var(--text)" />
    <path v-for="(d, i) in arrows" :key="i" class="arrow" :d="d" fill="var(--tint-home)" />
  </svg>
</template>

<style scoped>
.scene { display: block; width: 250px; height: 125px; margin: 0 auto; shape-rendering: crispEdges; }
/* One lit arrow at a time, left to right, in whole steps like the pixels. */
.moving .arrow { animation: lit 0.9s steps(1) infinite; opacity: 0.25; }
.moving .arrow:nth-of-type(4) { animation-delay: 0.3s; }
.moving .arrow:nth-of-type(5) { animation-delay: 0.6s; }
@keyframes lit { 0% { opacity: 1; } 33% { opacity: 0.25; } }
@media (prefers-reduced-motion: reduce) { .moving .arrow { animation: none; opacity: 1; } }
</style>
