<!-- A section of the app, opened like a program: one window on the desktop,
     its title bar on top, the section inside. In themes other than retro the
     frame and bar step aside and the section fills the stage as before. -->
<script setup lang="ts">
import type { Section } from "../lib/icons";
import TitleBar from "./TitleBar.vue";

defineProps<{ of: Section; title: string; max: boolean }>();
const emit = defineEmits<{ minimize: []; maximize: []; close: [] }>();
</script>

<template>
  <section class="win" :class="{ 'win-max': max }">
    <TitleBar
      class="win-bar"
      :of="of"
      :title="title"
      controls="live"
      @minimize="emit('minimize')"
      @maximize="emit('maximize')"
      @close="emit('close')"
    />
    <div class="win-body"><slot /></div>
  </section>
</template>

<style scoped>
.win {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  background: var(--window-bg);
  overflow: hidden;
}
.win-bar { display: none; }
.win-body { position: relative; flex: 1; min-height: 0; }

:global([data-theme="retro"] .win) {
  inset: var(--s5) var(--s6) calc(var(--s5) + 4px) var(--s5);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius-lg);
  box-shadow: var(--lift-panel);
}
:global([data-theme="retro"] .win.win-max) { inset: 0; border-width: 0 0 0 0; border-radius: 0; box-shadow: none; }
:global([data-theme="retro"] .win-bar) { display: flex; }
</style>
