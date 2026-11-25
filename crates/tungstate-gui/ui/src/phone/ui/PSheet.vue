<!-- A bottom sheet over a dimmed screen: a short choice or a question.
     Tapping the dim closes it, as every phone's sheets do. -->
<script setup lang="ts">
import { useBack } from "../lib/back";

defineProps<{ title: string; says?: string }>();
const emit = defineEmits<{ close: [] }>();
// Android's back closes the sheet first, before anything under it moves.
useBack(() => (emit("close"), true));
</script>

<template>
  <div class="veil" @click="$emit('close')"></div>
  <div class="sheet" role="dialog" :aria-label="title">
    <div class="grab"></div>
    <h3>{{ title }}</h3>
    <p v-if="says" class="says">{{ says }}</p>
    <slot />
  </div>
</template>

<style scoped>
.veil { position: fixed; inset: 0; background: var(--veil); z-index: 5; }
.sheet {
  position: fixed; left: 0; right: 0; bottom: 0; z-index: 6;
  display: flex; flex-direction: column; gap: 12px;
  /* Clear of the home bar on iPhone and the gesture bar on Android. */
  padding: 10px 20px calc(env(safe-area-inset-bottom, 0px) + 22px);
  background: var(--panel); border-top: 2px solid var(--edge); border-radius: 16px 16px 0 0;
}
.grab { width: 44px; height: 5px; margin: 0 auto 2px; border-radius: 3px; background: var(--edge); }
h3 { margin: 0; font-size: 24px; font-weight: 800; }
.says { margin: -4px 0 4px; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
</style>
