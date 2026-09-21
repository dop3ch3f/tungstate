<!-- Something the screen needs to say. Base carries the container, so a bare
     notice cannot render as an indented stray sentence the way slice 7b's did
     in three views at once. -->
<script setup lang="ts">
const props = withDefaults(
  defineProps<{ tone?: "plain" | "hold" | "bad" }>(),
  { tone: "plain" },
);
const TONE = { plain: "said-plain", hold: "said-hold", bad: "said-bad" } as const;
</script>

<template>
  <p class="said" :class="TONE[props.tone]"><slot /></p>
</template>

<style scoped>
.said {
  font-size: var(--small);
  line-height: 1.55;
  margin: 0;
  padding: var(--s2) var(--s3);
  border-left: 2px solid var(--edge);
  color: var(--text-quiet);
}
.said-plain { /* the default, already drawn above */ }
.said-hold { border-left-color: var(--hold); }
.said-bad { border-left-color: var(--bad); color: var(--text); }
</style>
