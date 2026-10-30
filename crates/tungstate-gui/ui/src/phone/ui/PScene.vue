<!-- The two wide pixel scenes from the approved Free up screens, 24 by 12
     squares: a phone sending to a NAS, and the NAS holding it all. Wide, so
     not drawn by the square icon renderer. -->
<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{ of: "send" | "done" }>();

const SCENES = {
  send: [
    "........................", ".xxxxx..............xxxx", ".x...x.............xoooo", ".x.o.x.............xxxxx",
    ".x...x...x..x..x...xoooo", ".x...x....x..x..x..xxxxx", ".x...x...x..x..x...xoooo", ".x...x.............xxxxx",
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
</script>

<template>
  <svg class="scene" viewBox="0 0 24 12" aria-hidden="true">
    <path :d="second" fill="var(--tint-drain)" />
    <path :d="ink" fill="var(--text)" />
  </svg>
</template>

<style scoped>
.scene { display: block; width: 250px; height: 125px; margin: 0 auto; shape-rendering: crispEdges; }
</style>
