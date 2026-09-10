<script setup lang="ts">
import { ref, computed, watch, onMounted } from "vue";
import { api, bytes, kind, mark, shortPath, type Entry, type Place } from "../api";

const props = defineProps<{ start: string; active: boolean; places: Place[] }>();
const emit = defineEmits<{
  focus: [];
  selection: [names: string[], total: number];
  located: [path: string];
}>();

type Field = "name" | "kind" | "size" | "modified";

const path = ref(props.start);
const draft = ref(props.start);
const editing = ref(false);
const entries = ref<Entry[]>([]);
const parent = ref<string | null>(null);
const chosen = ref<Set<string>>(new Set());
const error = ref("");
const loading = ref(false);
const sort = ref<{ field: Field; dir: 1 | -1 }>({ field: "name", dir: 1 });

// Folders stay above files whatever the sort, so navigation never has to be
// hunted for. The chosen field orders within each group.
const shown = computed(() => {
  const { field, dir } = sort.value;
  const compare = (a: Entry, b: Entry) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    let r = 0;
    if (field === "name") r = a.name.localeCompare(b.name, undefined, { numeric: true });
    else if (field === "kind") r = kind(a).localeCompare(kind(b)) || a.name.localeCompare(b.name);
    else if (field === "size") r = a.size - b.size;
    else r = (a.modified ?? 0) - (b.modified ?? 0);
    return r * dir;
  };
  return [...entries.value].sort(compare);
});

const total = computed(() =>
  entries.value.filter((e) => chosen.value.has(e.name)).reduce((n, e) => n + e.size, 0),
);
const allChosen = computed(() => entries.value.length > 0 && chosen.value.size === entries.value.length);
const someChosen = computed(() => chosen.value.size > 0 && !allChosen.value);

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

function by(field: Field) {
  sort.value =
    sort.value.field === field
      ? { field, dir: sort.value.dir === 1 ? -1 : 1 }
      : { field, dir: field === "size" || field === "modified" ? -1 : 1 };
}

function arrow(field: Field) {
  if (sort.value.field !== field) return "";
  return sort.value.dir === 1 ? "▲" : "▼";
}

const anchor = ref<number | null>(null);

// Clicking anywhere on a row toggles it. With checkboxes on screen that is the
// least surprising behaviour; shift still extends a run.
function toggle(entry: Entry, index: number, event?: MouseEvent) {
  emit("focus");
  if (event?.shiftKey && anchor.value !== null) {
    const [from, to] = [Math.min(anchor.value, index), Math.max(anchor.value, index)];
    const run = shown.value.slice(from, to + 1).map((e) => e.name);
    const adding = !chosen.value.has(entry.name);
    for (const name of run) {
      if (adding) chosen.value.add(name);
      else chosen.value.delete(name);
    }
  } else {
    if (chosen.value.has(entry.name)) chosen.value.delete(entry.name);
    else chosen.value.add(entry.name);
    anchor.value = index;
  }
  chosen.value = new Set(chosen.value);
  push();
}

function toggleAll() {
  chosen.value = allChosen.value ? new Set() : new Set(entries.value.map((e) => e.name));
  push();
}

function open(entry: Entry) {
  if (entry.is_dir) load(entry.path);
}

watch(() => props.start, (to) => { if (to !== path.value) load(to); });
onMounted(() => load(props.start));

defineExpose({ reload: () => load(path.value), path });
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
      <input class="path" :value="editing ? draft : shortPath(path)" :title="path" spellcheck="false"
             @focus="editing = true; draft = path"
             @input="(e) => (draft = (e.target as HTMLInputElement).value)"
             @keyup.enter="editing = false; load(draft)"
             @blur="editing = false; draft = path" />
    </div>

    <div class="listing">
      <div v-if="error" class="notice bad" style="margin: 10px 12px">{{ error }}</div>

      <template v-else>
        <div class="head chrome">
          <input class="tick" type="checkbox" :checked="allChosen"
                 :indeterminate.prop="someChosen" :disabled="!entries.length"
                 :title="allChosen ? 'Select none' : 'Select all'"
                 @change="toggleAll" />
          <button class="sorter" @click="by('name')">Name <i>{{ arrow("name") }}</i></button>
          <button class="sorter" @click="by('kind')">Type <i>{{ arrow("kind") }}</i></button>
          <button class="sorter num" @click="by('size')">Size <i>{{ arrow("size") }}</i></button>
          <button class="sorter num" @click="by('modified')">Modified <i>{{ arrow("modified") }}</i></button>
        </div>

        <div v-if="loading" class="empty">Reading…</div>
        <div v-else-if="!entries.length" class="empty">This folder is empty.</div>

        <div v-for="(entry, i) in shown" :key="entry.path"
             class="row" :class="{ chosen: chosen.has(entry.name), dir: entry.is_dir }"
             @click="toggle(entry, i, $event)" @dblclick="open(entry)">
          <input class="tick" type="checkbox" :checked="chosen.has(entry.name)"
                 @click.stop @change="toggle(entry, i)" />
          <div class="name">
            <span class="mark">{{ mark(entry) }}</span>
            <span class="label">{{ entry.name }}</span>
          </div>
          <div class="kind">{{ kind(entry) }}</div>
          <div class="num">{{ entry.is_dir ? "—" : bytes(entry.size) }}</div>
          <div class="num">{{ entry.modified ? new Date(entry.modified).toLocaleDateString() : "—" }}</div>
        </div>
      </template>
    </div>
  </section>
</template>
