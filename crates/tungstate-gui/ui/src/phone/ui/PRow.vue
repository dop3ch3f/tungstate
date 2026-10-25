<!-- A row in a list. Pressable rows show it with a chevron, never a shadow,
     so a list stays one object (the approved rule). -->
<script setup lang="ts">
withDefaults(defineProps<{ title: string; sub?: string; end?: string; chevron?: boolean }>(), { chevron: false });
defineEmits<{ press: [] }>();
</script>

<template>
  <component :is="chevron ? 'button' : 'div'" class="row" @click="chevron && $emit('press')">
    <slot name="lead" />
    <span class="text">
      <b>{{ title }}</b>
      <span v-if="sub" class="sub">{{ sub }}</span>
    </span>
    <span v-if="end" class="end">{{ end }}</span>
    <span v-if="chevron" class="chev" aria-hidden="true">›</span>
  </component>
</template>

<style scoped>
.row {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  min-height: 64px;
  padding: 12px 14px;
  font: inherit;
  text-align: left;
  color: var(--text);
  background: var(--panel);
  border: 0;
  border-top: 2px solid var(--edge);
}
.row:first-child { border-top: 0; }
button.row:active { background: var(--surface-hover); }
.text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
b { font-size: 16px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sub { font-size: 14px; color: var(--text-quiet); }
.end { font-size: 16px; font-weight: 800; flex: none; }
.chev { font-size: 22px; font-weight: 800; line-height: 1; flex: none; }
</style>
