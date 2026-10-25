<!-- The phone's buttons. `main` is filled with the section's colour; `quiet`
     is paper. Both carry the hard shadow, which collapses when pressed so the
     button feels pushed into the page. -->
<script setup lang="ts">
withDefaults(defineProps<{ tint?: string; quiet?: boolean; busy?: boolean; disabled?: boolean }>(), {
  tint: "var(--tint-drain)", quiet: false, busy: false, disabled: false,
});
defineEmits<{ press: [] }>();
</script>

<template>
  <button
    class="btn"
    :class="{ quiet: quiet }"
    :style="quiet ? undefined : { background: tint }"
    :disabled="disabled || busy"
    :aria-busy="busy || undefined"
    @click="$emit('press')"
  >
    <slot />
  </button>
</template>

<style scoped>
.btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  min-height: 58px;
  padding: 0 16px;
  font: inherit;
  font-size: 18px;
  font-weight: 800;
  color: #ffffff;
  border: 2px solid var(--edge);
  border-radius: 8px;
  box-shadow: 4px 4px 0 var(--edge);
  transition: transform 60ms steps(2), box-shadow 60ms steps(2);
}
.btn:active:not(:disabled) { transform: translate(4px, 4px); box-shadow: 0 0 0 var(--edge); }
.quiet { color: var(--text); background: var(--panel); font-weight: 700; font-size: 16px; min-height: 52px; box-shadow: 3px 3px 0 var(--edge); }
.btn:disabled { opacity: 0.55; }
.btn[aria-busy] { opacity: 0.8; }
</style>
