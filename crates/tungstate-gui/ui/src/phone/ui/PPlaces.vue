<!-- Where on this phone to look: the usual folders, those that exist, one to
     choose. Duplicates and Organise both start here. -->
<script setup lang="ts">
import { onMounted, ref } from "vue";
import { transfers } from "../../engine/commands";
import { placesUnder } from "../lib/organise";
import { useStorage } from "../state/useStorage";
import PThumb from "./PThumb.vue";

const props = defineProps<{ wanted: "dupes" | "organise"; modelValue: string | null }>();
const emit = defineEmits<{ "update:modelValue": [string] }>();
const store = useStorage();
const places = ref<{ label: string; path: string; where: string }[]>([]);

onMounted(async () => {
  if (!store.volumes.value.length) await store.load();
  const internal = store.volumes.value.find((v) => !v.removable) ?? store.volumes.value[0];
  if (!internal) return;
  const offered = placesUnder(internal.path, props.wanted);
  // Asked together: five quick listings rather than five in a row.
  const there = await Promise.all(offered.map((p) => transfers.browse(p.path).then(() => true, () => false)));
  places.value = offered
    .filter((_, i) => there[i])
    .map((p) => ({ ...p, where: p.path === internal.path ? internal.label : `${internal.label} › ${p.path.slice(internal.path.length + 1).split("/").join(" › ")}` }));
  if (!props.modelValue && places.value[0]) emit("update:modelValue", places.value[0].path);
});
</script>

<template>
  <div class="list">
    <button v-for="p in places" :key="p.path" class="pick" :class="{ on: modelValue === p.path }" @click="emit('update:modelValue', p.path)">
      <span class="radio" :class="{ on: modelValue === p.path }" aria-hidden="true"></span>
      <PThumb :name="p.label" :path="p.path" is-dir :size="34" />
      <span class="text"><b>{{ p.label }}</b><span class="quiet">{{ p.where }}</span></span>
    </button>
    <p v-if="!places.length" class="none">Looking for folders…</p>
  </div>
</template>

<style scoped>
.list { overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.pick { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 64px; padding: 9px 14px; font: inherit; text-align: left; color: var(--text); background: var(--panel); border: 0; border-top: 2px solid var(--edge); }
.pick:first-child { border-top: 0; }
.pick.on { background: var(--surface-hover); }
.radio { position: relative; flex: none; width: 26px; height: 26px; background: var(--panel); border: 2px solid var(--edge); border-radius: 99px; }
.radio.on::after { content: ""; position: absolute; inset: 4px; border-radius: 99px; background: var(--edge); }
.text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.text b { font-size: 16px; }
.quiet { font-size: 14px; color: var(--text-quiet); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.none { margin: 0; padding: 16px; font-size: 15px; color: var(--text-quiet); }
</style>
