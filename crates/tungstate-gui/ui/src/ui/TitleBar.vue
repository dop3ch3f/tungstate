<!-- A window's title bar: its icon and name on the left, the three window
     controls on the right. In the retro theme it is drawn as a bar; in the
     others it is a plain heading and the controls are not shown.

     Controls are drawn only where they do something. Home's small windows are
     shortcuts, not programs, so they have none: a button that cannot be
     pressed is worse than no button. -->
<script setup lang="ts">
import type { Section } from "../lib/icons";
import Tile from "./Tile.vue";

const props = withDefaults(
  defineProps<{ of: Section; title: string; controls?: "live" | "none" }>(),
  { controls: "none" },
);
const emit = defineEmits<{ minimize: []; maximize: []; close: [] }>();

function press(id: "minimize" | "maximize" | "close") {
  if (id === "minimize") emit("minimize");
  else if (id === "maximize") emit("maximize");
  else emit("close");
}

const CONTROLS = [
  { id: "minimize", label: "Minimize", d: "M4 11.5h8" },
  { id: "maximize", label: "Maximize", d: "M4 4h8v8H4z" },
  { id: "close", label: "Close", d: "M4.5 4.5l7 7M11.5 4.5l-7 7" },
] as const;
</script>

<template>
  <header class="tb">
    <Tile :of="props.of" :size="20" />
    <span class="tb-title">{{ props.title }}</span>
    <span class="tb-extra"><slot /></span>
    <span class="tb-ctl" v-if="props.controls === 'live'">
      <button
        v-for="c in CONTROLS"
        :key="c.id"
        class="tb-b"
        :class="{ 'tb-close': c.id === 'close' }"
        :aria-label="c.label"
        :title="c.label"
        @click="press(c.id)"
      >
        <svg viewBox="0 0 16 16"><path :d="c.d" /></svg>
      </button>
    </span>
  </header>
</template>

<style scoped>
.tb { display: flex; align-items: center; gap: var(--s2); min-width: 0; }
.tb-title { font-size: var(--body); font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.tb-extra { margin-left: auto; display: flex; align-items: center; gap: var(--s3); font-size: var(--small); }
.tb-ctl { display: none; gap: 5px; flex: none; }
.tb-b {
  display: grid;
  place-items: center;
  width: 22px;
  height: 20px;
  padding: 0;
  font: inherit;
  color: var(--text);
  background: var(--panel);
  border: var(--bw) solid var(--edge);
  border-radius: 3px;
  box-shadow: 1px 1px 0 var(--edge);
}
.tb-b { cursor: pointer; }
button.tb-b:hover { background: var(--surface-hover); }
button.tb-close:hover { background: var(--tint-folder); color: var(--tint-ink); }
button.tb-b:active { transform: translate(1px, 1px); box-shadow: none; }
.tb-b svg { width: 12px; height: 12px; fill: none; stroke: currentColor; stroke-width: 1.6; stroke-linecap: round; }

/* Retro: a real title bar. */
:global([data-theme="retro"] .tb) {
  background: var(--surface-raised);
  border-bottom: var(--bw) solid var(--edge);
  padding: 6px 7px 6px var(--s3);
}
:global([data-theme="retro"] .tb-title) { font-size: var(--small); font-weight: 700; }
:global([data-theme="retro"] .tb-ctl) { display: flex; }
</style>
