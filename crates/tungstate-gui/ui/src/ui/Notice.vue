<!-- Something the screen needs to say. Base carries the container, so a bare
     notice cannot render as an indented stray sentence the way slice 7b's did
     in three views at once. A warning and a failure carry an icon drawn at
     text size and set on the first line, so it reads as part of the sentence
     rather than a speck in the corner. -->
<script setup lang="ts">
import AlertIcon from "./AlertIcon.vue";

const props = withDefaults(
  defineProps<{ tone?: "plain" | "hold" | "bad" }>(),
  { tone: "plain" },
);
const TONE = { plain: "said-plain", hold: "said-hold", bad: "said-bad" } as const;
/** What a warning and a failure share, so retro can draw both as one bar. */
const ALERT = { plain: null, hold: "said-alert", bad: "said-alert" } as const;
</script>

<template>
  <p class="said" :class="[TONE[props.tone], ALERT[props.tone]]">
    <AlertIcon v-if="props.tone !== 'plain'" :tone="props.tone" />
    <span class="said-text"><slot /></span>
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
}
.said-plain { /* the default, already drawn above */ }
.said-text { min-width: 0; }

.said-hold { background: none; border-left: none; padding-left: 0; color: var(--hold); --alert-knock: var(--field); }
.said-bad { border-left-color: var(--bad); color: var(--text); --alert-icon: var(--bad); }

/* Retro: a warning and a failure are the same outlined bar with a hard
   shadow, yellow for a warning and red for a failure, the icon in ink. */
:global([data-theme="retro"] .said) { border-left-width: var(--bw); }
:global([data-theme="retro"] .said-alert) {
  border: var(--bw) solid var(--warn-ink);
  border-radius: var(--radius);
  padding: var(--s2) var(--s3);
  box-shadow: var(--lift);
  /* A link inside is read on the bar too, whichever tone the theme is. */
  --text: var(--warn-ink);
  --text-quiet: var(--warn-ink);
  --alert-icon: var(--warn-ink);
}
:global([data-theme="retro"] .said-hold) { background: var(--warn-bar); color: var(--warn-ink); --alert-knock: var(--warn-bar); }
:global([data-theme="retro"] .said-bad) { background: var(--alarm-bar); color: var(--warn-ink); --alert-knock: var(--alarm-bar); }
</style>
