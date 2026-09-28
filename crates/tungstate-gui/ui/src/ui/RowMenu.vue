<!-- A row's lesser actions behind "…". The one or two a row is for stay in
     it; the rest, and anything that deletes, are a deliberate click further.
     Closes on a pick, a click elsewhere, or Escape. -->
<script setup lang="ts">
import { onBeforeUnmount, ref } from "vue";

export interface MenuItem {
  id: string;
  label: string;
  /** Loses something: drawn in the colour for that. */
  danger?: boolean;
  busy?: boolean;
}

const props = defineProps<{ items: MenuItem[]; label?: string }>();
const emit = defineEmits<{ pick: [id: string] }>();
const open = ref(false);
const root = ref<HTMLElement | null>(null);

function outside(e: MouseEvent) {
  if (root.value && !root.value.contains(e.target as Node)) close();
}
function escape(e: KeyboardEvent) {
  if (e.key === "Escape") close();
}
function show() {
  open.value = true;
  document.addEventListener("mousedown", outside);
  document.addEventListener("keydown", escape);
}
function close() {
  open.value = false;
  document.removeEventListener("mousedown", outside);
  document.removeEventListener("keydown", escape);
}
onBeforeUnmount(close);

function pick(id: string) {
  close();
  emit("pick", id);
}
</script>

<template>
  <span ref="root" class="rm">
    <button
      class="rm-open"
      :aria-expanded="open"
      aria-haspopup="menu"
      :aria-label="props.label ?? 'More actions'"
      :title="props.label ?? 'More actions'"
      @click="open ? close() : show()"
    >…</button>
    <span class="rm-list" v-if="open" role="menu">
      <button
        v-for="item in props.items"
        :key="item.id"
        role="menuitem"
        class="rm-item"
        :class="{ 'rm-danger': item.danger }"
        :disabled="item.busy"
        :aria-busy="item.busy || undefined"
        @click="pick(item.id)"
      >{{ item.label }}</button>
    </span>
  </span>
</template>

<style scoped>
.rm { position: relative; display: inline-flex; }
.rm-open {
  font: inherit;
  font-size: var(--body);
  line-height: 1;
  color: var(--text-quiet);
  background: none;
  border: var(--bw) solid transparent;
  border-radius: var(--radius);
  padding: 2px 8px 6px;
  cursor: pointer;
}
.rm-open:hover,
.rm-open[aria-expanded="true"] { color: var(--text); border-color: var(--edge); background: var(--surface-hover); }
/* It floats over the rows below, so it is one of the few things that keeps a
   shadow. */
.rm-list {
  position: absolute;
  top: calc(100% + var(--s1));
  right: 0;
  z-index: 5;
  display: flex;
  flex-direction: column;
  min-width: 170px;
  padding: var(--s1) 0;
  background: var(--sheet);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius);
  box-shadow: var(--lift-panel);
}
.rm-item {
  font: inherit;
  font-size: var(--small);
  text-align: left;
  color: var(--text);
  background: none;
  border: none;
  padding: 6px var(--s3);
  cursor: pointer;
}
.rm-item:hover:not(:disabled) { background: var(--surface-hover); }
.rm-item[aria-busy="true"] { cursor: progress; opacity: 0.6; }
.rm-danger { color: var(--bad); }
</style>
