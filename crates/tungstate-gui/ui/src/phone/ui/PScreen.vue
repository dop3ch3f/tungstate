<!-- One phone screen: an optional back link, a large title, a scrolling body,
     and the main action pinned in the thumb zone above the tabs. -->
<script setup lang="ts">
defineProps<{ title: string; back?: string }>();
defineEmits<{ back: [] }>();
</script>

<template>
  <section class="screen">
    <div class="top">
      <button v-if="back" class="back" @click="$emit('back')">‹ {{ back }}</button>
      <div class="head">
        <h1>{{ title }}</h1>
        <slot name="corner" />
      </div>
    </div>
    <div class="body"><slot /></div>
    <div v-if="$slots.act" class="act"><slot name="act" /></div>
  </section>
</template>

<style scoped>
.screen { flex: 1; min-height: 0; display: flex; flex-direction: column; }
.top { flex: none; padding: 6px 20px 0; }
.back {
  min-height: 44px;
  padding: 0;
  font: inherit;
  font-size: 17px;
  font-weight: 700;
  color: var(--text);
  background: none;
  border: 0;
}
.head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin: 4px 0 14px; }
h1 { font-size: 32px; line-height: 1.08; font-weight: 800; letter-spacing: -0.6px; margin: 0; overflow-wrap: anywhere; }
.body { flex: 1; min-height: 0; overflow-y: auto; padding: 2px 20px 20px; -webkit-overflow-scrolling: touch; }
.act { flex: none; padding: 10px 20px 12px; }
</style>
