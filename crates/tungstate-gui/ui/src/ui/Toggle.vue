<!-- An on/off setting that takes effect at once, drawn as a switch rather
     than a tick: a tick reads as part of a form waiting for Save. -->
<script setup lang="ts">
const props = defineProps<{ on: boolean; label: string; disabled?: boolean }>();
const emit = defineEmits<{ change: [on: boolean] }>();
</script>

<template>
  <button
    class="tg"
    :class="{ 'tg-on': props.on }"
    role="switch"
    :aria-checked="props.on"
    :aria-label="props.label"
    :disabled="props.disabled"
    @click="emit('change', !props.on)"
  ><span class="tg-knob" aria-hidden="true"></span></button>
</template>

<style scoped>
.tg {
  position: relative;
  flex: none;
  width: 42px;
  height: 24px;
  padding: 0;
  border: var(--bw) solid var(--edge);
  border-radius: 999px;
  background: var(--surface-raised);
  cursor: pointer;
  transition: background var(--quick) var(--ease);
}
.tg-knob {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--text);
  transition: transform var(--quick) var(--ease);
}
.tg-on { background: var(--control); }
.tg-on .tg-knob { transform: translateX(18px); background: var(--control-ink); }
.tg:disabled { opacity: 0.5; cursor: default; }
/* Retro: an outlined slot with a hard shadow, like every other control. */
:global([data-theme="retro"] .tg) { box-shadow: var(--lift); }
</style>
