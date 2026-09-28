<!-- Something the screen needs to say, at one of three levels: a note, a
     warning, a failure. All three are the same bar with an icon drawn at text
     size on the first line, so a screen never mixes a quote, a banner and a
     stray coloured sentence for things that mean the same. No hard shadow:
     a notice is read, not pressed. -->
<script setup lang="ts">
import AlertIcon from "./AlertIcon.vue";

const props = withDefaults(
  defineProps<{ tone?: "plain" | "hold" | "bad" }>(),
  { tone: "plain" },
);
const TONE = { plain: "said-plain", hold: "said-hold", bad: "said-bad" } as const;
</script>

<template>
  <p class="said" :class="TONE[props.tone]">
    <AlertIcon :tone="props.tone" />
    <span class="said-text"><slot /></span>
    <span class="said-act" v-if="$slots.act"><slot name="act" /></span>
  </p>
</template>

<style scoped>
.said {
  display: flex;
  align-items: flex-start;
  gap: var(--s2);
  font-size: var(--small);
  line-height: 1.55;
  margin: 0;
  padding: var(--s2) var(--s3);
  background: var(--panel);
  border-left: 3px solid var(--edge);
  border-radius: var(--radius);
  color: var(--text-quiet);
  --alert-icon: var(--text-faint);
  --alert-knock: var(--panel);
}
.said-plain { /* the default, already drawn above */ }
.said-text { flex: 1; min-width: 0; }
/* The one step that deals with it, at the right edge, never mid-sentence. */
.said-act { flex: none; display: flex; align-items: center; gap: var(--s3); }
.said-hold { border-left-color: var(--hold); color: var(--text); --alert-icon: var(--hold); }
.said-bad { border-left-color: var(--bad); color: var(--text); --alert-icon: var(--bad); }

/* Retro: one outlined bar in ink for all three, cream for a note, yellow for
   a warning, red for a failure, the icon in ink. */
:global([data-theme="retro"] .said) {
  border: var(--bw) solid var(--warn-ink);
  padding: var(--s2) var(--s3);
  background: var(--surface-raised);
  color: var(--text);
  --alert-icon: var(--text);
  --alert-knock: var(--surface-raised);
}
:global([data-theme="retro"] .said-hold) {
  color: var(--warn-ink);
  /* A link inside is read on the bar too, whichever tone the theme is. */
  --text: var(--warn-ink);
  --text-quiet: var(--warn-ink);
  --alert-icon: var(--warn-ink);
}
:global([data-theme="retro"] .said-bad) {
  color: var(--warn-ink);
  /* A link inside is read on the bar too, whichever tone the theme is. */
  --text: var(--warn-ink);
  --text-quiet: var(--warn-ink);
  --alert-icon: var(--warn-ink);
}
:global([data-theme="retro"] .said-hold) { background: var(--warn-bar); --alert-knock: var(--warn-bar); }
:global([data-theme="retro"] .said-bad) { background: var(--alarm-bar); --alert-knock: var(--alarm-bar); }
</style>
