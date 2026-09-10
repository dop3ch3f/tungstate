<script setup lang="ts">
import { ref, computed, watch, onMounted } from "vue";
import { api, bytes, mark, shortPath, type Entry, type Place } from "../api";

const props = defineProps<{ start: string; active: boolean; places: Place[] }>();
const emit = defineEmits<{
  focus: [];
  selection: [names: string[], total: number];
  located: [path: string];
}>();

const path = ref(props.start);
const draft = ref(props.start);
const editing = ref(false);
const entries = ref<Entry[]>([]);
const parent = ref<string | null>(null);
const chosen = ref<Set<string>>(new Set());
const error = ref("");
const loading = ref(false);

const total = computed(() =>
  entries.value.filter((e) => chosen.value.has(e.name)).reduce((n, e) => n + e.size, 0),
);

async function load(to: string) {
  loading.value = true;
  try {
    const listing = await api.browse(to);
    entries.value = listing.entries;
    parent.value = listing.parent;
    path.value = listing.path;
    draft.value = listing.path;
    chosen.value = new Set();
    error.value = "";
    emit("located", listing.path);
    push();
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

function push() {
  emit("selection", [...chosen.value], total.value);
}

function open(entry: Entry) {
  if (entry.is_dir) load(entry.path);
}

// Cmd or Ctrl adds to the selection; a plain click replaces it. Shift extends,
// which is what any file list is expected to do.
const anchor = ref<number | null>(null);
function pick(entry: Entry, index: number, event: MouseEvent) {
  emit("focus");
  if (event.metaKey || event.ctrlKey) {
    chosen.value.has(entry.name) ? chosen.value.delete(entry.name) : chosen.value.add(entry.name);
    anchor.value = index;
  } else if (event.shiftKey && anchor.value !== null) {
    const [from, to] = [Math.min(anchor.value, index), Math.max(anchor.value, index)];
    chosen.value = new Set(entries.value.slice(from, to + 1).map((e) => e.name));
  } else {
    chosen.value = new Set([entry.name]);
    anchor.value = index;
  }
  chosen.value = new Set(chosen.value);
  push();
}

function selectAll() {
  chosen.value = new Set(entries.value.map((e) => e.name));
  push();
}

watch(() => props.start, (to) => { if (to !== path.value) load(to); });
onMounted(() => load(props.start));

defineExpose({ reload: () => load(path.value), path, selectAll });
</script>

<template>
  <section class="pane" :class="{ active: props.active }" @mousedown="emit('focus')">
    <div class="locbar chrome">
      <select :value="''" @change="(e) => load((e.target as HTMLSelectElement).value)" title="Go to">
        <option value="" disabled selected>Go to…</option>
        <option v-for="p in props.places" :key="p.path" :value="p.path">{{ p.label }}</option>
      </select>
      <button class="btn quiet small" :disabled="!parent" title="Up one folder"
              @click="parent && load(parent)">↑</button>
      <input class="path" :value="editing ? draft : shortPath(path)" :title="path"
             spellcheck="false"
             @focus="editing = true; draft = path"
             @input="(e) => (draft = (e.target as HTMLInputElement).value)"
             @keyup.enter="editing = false; load(draft)"
             @blur="editing = false; draft = path" />
    </div>

    <div class="listing">
      <div v-if="error" class="notice bad" style="margin: 10px 12px">{{ error }}</div>

      <template v-else>
        <div class="head chrome">
          <div>Name</div><div class="num">Size</div><div class="num">Modified</div>
        </div>

        <div v-if="loading" class="empty">Reading…</div>
        <div v-else-if="!entries.length" class="empty">This folder is empty.</div>

        <div v-for="(entry, i) in entries" :key="entry.path"
             class="row" :class="{ chosen: chosen.has(entry.name), dir: entry.is_dir }"
             @click="pick(entry, i, $event)" @dblclick="open(entry)">
          <div class="name">
            <span class="mark">{{ mark(entry) }}</span>
            <span class="label">{{ entry.name }}</span>
          </div>
          <div class="num">{{ entry.is_dir ? "—" : bytes(entry.size) }}</div>
          <div class="num">{{ entry.modified ? new Date(entry.modified).toLocaleDateString() : "—" }}</div>
        </div>
      </template>
    </div>
  </section>
</template>
