<!-- How a section works, in three tiles, for somebody who has not used it
     yet. Shown until the section has done something once; after that its own
     history says the same thing better. -->
<script setup lang="ts">
import Pixels from "./Pixels.vue";
import type { Art } from "../lib/pixels";

defineProps<{ steps: { art: Art; title: string; line: string }[] }>();
</script>

<template>
  <ol class="steps">
    <li v-for="(step, index) in steps" :key="step.title" class="step">
      <div class="step-top">
        <span class="step-n">{{ index + 1 }}</span>
        <Pixels class="step-art" :of="step.art" :size="36" />
      </div>
      <b class="step-title">{{ step.title }}</b>
      <span class="step-line">{{ step.line }}</span>
    </li>
  </ol>
</template>

<style scoped>
.steps { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--s3); }
.step {
  display: flex;
  flex-direction: column;
  gap: var(--s2);
  padding: var(--s3);
  background: var(--panel);
  border: 1px solid var(--edge);
  border-radius: var(--radius-lg);
}
.step-top { display: flex; align-items: center; gap: var(--s3); }
.step-n {
  width: 22px;
  height: 22px;
  display: grid;
  place-items: center;
  border: 1px solid var(--edge);
  border-radius: 50%;
  font-size: var(--fine);
  font-weight: 700;
  background: var(--surface-raised);
}
/* The art takes the colour of the section it is standing in. */
.step-art { color: var(--control); }
.step-title { font-size: var(--small); }
.step-line { font-size: var(--fine); color: var(--text-faint); line-height: 1.45; }
:global([data-theme="retro"] .step) { border-width: var(--bw); box-shadow: var(--lift); }
:global([data-theme="retro"] .step-n) { border-width: var(--bw); }
@media (max-width: 760px) { .steps { grid-template-columns: 1fr; } }
</style>
