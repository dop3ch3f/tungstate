<!-- One side of the browser. Two of these are how files get picked.
     The logic is slice 4e's, which was right: folders pinned above files
     whatever the sort, a whole-row click to tick, shift to extend a run. -->
<script setup lang="ts">
import { ref, computed, watch, onMounted } from "vue";
import { transfers } from "../../engine/commands";
import { bytes, kind, mark, shortPath, when } from "../../lib/format";
import type { Entry, Place } from "../../engine/types";
import Notice from "../../ui/Notice.vue";

const props = defineProps<{ start: string; active: boolean; places: Place[] }>();
const emit = defineEmits<{
  focus: [];
  selection: [names: string[], total: number];
  located: [path: string];
}>();

type Field = "name" | "kind" | "size" | "modified";

const here = ref(props.start);
const draft = ref(props.start);
const editing = ref(false);
const entries = ref<Entry[]>([]);
const parent = ref<string | null>(null);
const ticked = ref<Set<string>>(new Set());
const problem = ref("");
const loading = ref(false);
const sort = ref<{ field: Field; dir: 1 | -1 }>({ field: "name", dir: 1 });

// Folders stay above files whatever the sort, so navigation never has to be
// hunted for. The chosen field orders within each group.
const shown = computed(() => {
  const { field, dir } = sort.value;
  return [...entries.value].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    let r = 0;
    if (field === "name") r = a.name.localeCompare(b.name, undefined, { numeric: true });
    else if (field === "kind") r = kind(a).localeCompare(kind(b)) || a.name.localeCompare(b.name);
    else if (field === "size") r = a.size - b.size;
    else r = (a.modified ?? 0) - (b.modified ?? 0);
    return r * dir;
  });
});

const weight = computed(() =>
  entries.value.filter((e) => ticked.value.has(e.name)).reduce((n, e) => n + e.size, 0),
);
const allTicked = computed(() => entries.value.length > 0 && ticked.value.size === entries.value.length);
const someTicked = computed(() => ticked.value.size > 0 && !allTicked.value);

async function load(to: string) {
  loading.value = true;
  try {
    const listing = await transfers.browse(to);
    entries.value = listing.entries;
    parent.value = listing.parent;
    here.value = listing.path;
    draft.value = listing.path;
    ticked.value = new Set();
    problem.value = "";
    emit("located", listing.path);
    push();
  } catch (e) {
    problem.value = String(e);
  } finally {
    loading.value = false;
  }
}

const push = () => emit("selection", [...ticked.value], weight.value);

function by(field: Field) {
  sort.value =
    sort.value.field === field
      ? { field, dir: sort.value.dir === 1 ? -1 : 1 }
      : { field, dir: field === "size" || field === "modified" ? -1 : 1 };
}
const arrow = (field: Field) =>
  sort.value.field !== field ? "" : sort.value.dir === 1 ? "▲" : "▼";

const anchor = ref<number | null>(null);

function tick(entry: Entry, index: number, event?: MouseEvent) {
  emit("focus");
  if (event?.shiftKey && anchor.value !== null) {
    const [from, to] = [Math.min(anchor.value, index), Math.max(anchor.value, index)];
    const adding = !ticked.value.has(entry.name);
    for (const name of shown.value.slice(from, to + 1).map((e) => e.name)) {
      if (adding) ticked.value.add(name);
      else ticked.value.delete(name);
    }
  } else {
    if (ticked.value.has(entry.name)) ticked.value.delete(entry.name);
    else ticked.value.add(entry.name);
    anchor.value = index;
  }
  ticked.value = new Set(ticked.value);
  push();
}

function tickAll() {
  ticked.value = allTicked.value ? new Set() : new Set(entries.value.map((e) => e.name));
  push();
}

watch(() => props.start, (to) => { if (to !== here.value) load(to); });
onMounted(() => load(props.start));

defineExpose({ reload: () => load(here.value), here });
</script>

<template>
  <section class="side" :class="{ lit: props.active }" @mousedown="emit('focus')">
    <header class="locline">
      <select class="jump" @change="load(($event.target as HTMLSelectElement).value)">
        <option value="">Go to…</option>
        <option v-for="place in props.places" :key="place.path" :value="place.path">
          {{ place.label }}
        </option>
      </select>
      <button class="up" :disabled="!parent" @click="parent && load(parent)" title="Up one">↑</button>
      <input
        class="path where-input"
        :value="editing ? draft : shortPath(here)"
        @focus="editing = true; draft = here"
        @blur="editing = false"
        @input="draft = ($event.target as HTMLInputElement).value"
        @keyup.enter="load(draft)"
      />
    </header>

    <Notice tone="bad" v-if="problem">{{ problem }}</Notice>

    <template v-else>
      <div class="cols">
        <input
          type="checkbox"
          :checked="allTicked"
          :indeterminate="someTicked"
          @change="tickAll()"
          aria-label="Tick everything here"
        />
        <button @click="by('name')">Name {{ arrow("name") }}</button>
        <button @click="by('kind')">Type {{ arrow("kind") }}</button>
        <button class="r" @click="by('size')">Size {{ arrow("size") }}</button>
        <button class="r" @click="by('modified')">Modified {{ arrow("modified") }}</button>
      </div>

      <div class="rolls">
        <p class="nothing" v-if="loading">Reading…</p>
        <p class="nothing" v-else-if="!entries.length">This folder is empty.</p>
        <div
          v-for="(entry, index) in shown"
          :key="entry.path"
          class="line"
          :class="{ on: ticked.has(entry.name), folder: entry.is_dir }"
          @click="tick(entry, index, $event)"
          @dblclick="entry.is_dir && load(entry.path)"
        >
          <input type="checkbox" :checked="ticked.has(entry.name)" tabindex="-1" @click.stop="tick(entry, index, $event)" />
          <span class="entryname"><span class="glyph">{{ mark(entry) }}</span><span class="nm">{{ entry.name }}</span></span>
          <span class="what">{{ kind(entry) }}</span>
          <span class="r num">{{ entry.is_dir ? "" : bytes(entry.size) }}</span>
          <span class="r num">{{ entry.modified ? when(entry.modified) : "" }}</span>
        </div>
      </div>
    </template>
  </section>
</template>

<style scoped>
.side {
  display: flex;
  flex-direction: column;
  min-width: 0;
  border: 1px solid var(--edge);
  border-radius: var(--radius);
  overflow: hidden;
}
.side.lit { border-color: var(--text-faint); }

.locline { display: flex; gap: var(--s2); padding: var(--s2); align-items: center; }
.jump, .up, .where-input {
  font: inherit;
  font-size: var(--small);
  background: none;
  color: var(--text-quiet);
  border: 1px solid var(--edge);
  border-radius: var(--radius);
  padding: 4px 7px;
}
.jump, .up { cursor: pointer; flex: none; }
.up:disabled { opacity: 0.4; cursor: default; }
/* Not the global `.path` word-break: this one ellipsizes on a single line. */
.where-input { flex: 1; min-width: 0; word-break: normal; text-overflow: ellipsis; }
.where-input:focus { color: var(--text); border-color: var(--text-faint); }

.cols, .line {
  display: grid;
  grid-template-columns: 22px minmax(0, 1fr) 86px 78px 104px;
  gap: var(--s2);
  align-items: center;
  padding: 0 var(--s2);
}
.cols {
  height: 26px;
  border-bottom: 1px solid var(--edge);
  font-size: var(--fine);
  color: var(--text-faint);
}
.cols button {
  font: inherit;
  font-size: var(--fine);
  background: none;
  border: none;
  color: inherit;
  padding: 0;
  text-align: left;
  cursor: pointer;
}
.cols .r, .line .r { text-align: right; }

.rolls { flex: 1; overflow-y: auto; }
.line { height: 26px; cursor: default; font-size: var(--small); }
.line:hover { background: var(--surface-hover); }
.line.on { background: var(--surface-raised); }
.entryname { display: flex; align-items: center; gap: var(--s2); min-width: 0; }
/* The ellipsis has to be on the text, not on the flex row around it: a flex
   container clips its children and never shows the dots. */
.nm { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
.glyph { color: var(--text-faint); flex: none; }
.line.folder .glyph { color: var(--text-quiet); }
.what, .line .num { color: var(--text-faint); font-size: var(--fine); white-space: nowrap; }
.nothing { font-size: var(--small); color: var(--text-faint); padding: var(--s4) var(--s2); margin: 0; }
</style>
