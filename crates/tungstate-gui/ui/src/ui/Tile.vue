<!-- A section's icon on a rounded square of its own colour, the way System
     Settings marks its panes. The only place a section's colour is a fill. -->
<script setup lang="ts">
import { ICON, type Section } from "../lib/icons";
import Pixels from "./Pixels.vue";

const props = withDefaults(defineProps<{ of: Section; size?: number }>(), { size: 22 });
// A lookup, because the CSS checker cannot follow a computed class name.
const TINT = { home: "t-home", folder: "t-folder", drain: "t-drain", history: "t-history", settings: "t-history" } as const;
</script>

<template>
  <span class="tile" :class="TINT[props.of]" :style="{ width: `${size}px`, height: `${size}px` }" aria-hidden="true">
    <svg class="line" viewBox="0 0 16 16"><path :d="ICON[props.of]" /></svg>
    <Pixels class="px-mark" :of="props.of" :size="Math.round(props.size * 0.66)" />
  </span>
</template>

<style scoped>
.tile {
  display: inline-grid;
  place-items: center;
  flex: none;
  border-radius: 24%;
  color: var(--tint-ink);
}
.line { width: 64%; height: 64%; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
.px-mark { display: none; }
/* Retro draws its marks as pixels; every other theme uses the line glyph. */
:global([data-theme="retro"]) .line { display: none; }
:global([data-theme="retro"]) .px-mark { display: block; }

.t-home { background: var(--tint-home); color: var(--tint-home-ink); }
.t-folder { background: var(--tint-folder); }
.t-drain { background: var(--tint-drain); }
.t-history { background: var(--tint-history); }
</style>
