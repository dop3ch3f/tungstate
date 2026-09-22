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
  background: var(--glass);
  border-left: 3px solid var(--edge);
  border-radius: var(--radius);
  color: var(--text-quiet);
}
.said-plain { /* the default, already drawn above */ }
.said-hold {
  background: none;
  border-left: none;
  padding: 0 0 0 18px;
  position: relative;
  color: var(--hold);
}
.said-hold::before {
  content: "!";
  position: absolute;
  left: 0;
  top: 2px;
  width: 13px;
  height: 13px;
  border-radius: 50%;
  background: var(--hold);
  color: var(--control-ink);
  font-size: 10px;
  font-weight: 800;
  line-height: 13px;
  text-align: center;
}
.said-bad { border-left-color: var(--bad); color: var(--text); }
</style>
