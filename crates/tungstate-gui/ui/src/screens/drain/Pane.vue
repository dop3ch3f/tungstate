<!-- One side of the browser. Two of these are how files get picked.
     The logic is slice 4e's, which was right: folders pinned above files
     whatever the sort, a whole-row click to tick, shift to extend a run. -->
<script setup lang="ts">
import { ref, computed, watch, onMounted } from "vue";
import { transfers } from "../../engine/commands";
import { bytes, kind, shortPath, when } from "../../lib/format";
import type { Entry, Place } from "../../engine/types";
import Notice from "../../ui/Notice.vue";
import { useTable } from "../../lib/table";

const props = defineProps<{ start: string; active: boolean; places: Place[] }>();
const emit = defineEmits<{
  focus: [];
  selection: [names: string[], total: number];
  located: [path: string];
}>();

const here = ref(props.start);
const draft = ref(props.start);
const editing = ref(false);
const entries = ref<Entry[]>([]);
const parent = ref<string | null>(null);
const ticked = ref<Set<string>>(new Set());
const problem = ref("");
const loading = ref(false);
// Folders stay above files whatever the sort, so navigation never has to be
// hunted for. The chosen column orders within each group.
const table = useTable(entries, {
  columns: [
    { key: "name", value: (e) => e.name },
    { key: "kind", value: (e) => kind(e) },
    { key: "size", value: (e) => e.size },
    { key: "modified", value: (e) => e.modified ?? 0 },
  ],
  text: (e) => e.name,
  pinned: (e) => e.is_dir,
});
const shown = table.shown;

const weight = computed(() =>
  entries.value.filter((e) => ticked.value.has(e.name)).reduce((n, e) => n + e.size, 0),
);
// Over what is showing: with a filter on, "tick everything" means everything
// you can see, not files the filter is hiding.
const allTicked = computed(() => shown.value.length > 0 && shown.value.every((e) => ticked.value.has(e.name)));
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
    table.query.value = "";
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

const by = (field: string) => table.by(field, field === "size" || field === "modified");
const arrow = (field: string) =>
  table.sort.value.key !== field ? "" : table.sort.value.dir === 1 ? "▲" : "▼";

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
  const next = new Set(ticked.value);
  for (const e of shown.value) {
    if (allTicked.value) next.delete(e.name);
    else next.add(e.name);
  }
  ticked.value = next;
  push();
}

watch(() => props.start, (to) => { if (to !== here.value) load(to); });
onMounted(() => load(props.start));

defineExpose({ reload: () => load(here.value), here });

// Kind label to badge colour. A lookup, because the CSS checker cannot follow
// a computed class name; anything unlisted is drawn as a document.
const KIND: Record<string, string> = {
  Folder: "k-folder",
  Video: "k-video",
  Image: "k-image",
  "Raw image": "k-image",
  Audio: "k-audio",
  Archive: "k-archive",
};
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
      <div class="sift" v-if="entries.length > 1">
        <input v-model="table.query.value" type="search" placeholder="Filter this folder" aria-label="Filter this folder" />
        <span class="sift-n" v-if="table.narrowed.value">{{ shown.length }} of {{ entries.length }}</span>
      </div>
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
        <p class="nothing" v-else-if="!shown.length">Nothing here matches that.</p>
        <div
          v-for="(entry, index) in shown"
          :key="entry.path"
          class="line"
          :class="{ on: ticked.has(entry.name) }"
          @click="tick(entry, index, $event)"
          @dblclick="entry.is_dir && load(entry.path)"
        >
          <input type="checkbox" :checked="ticked.has(entry.name)" tabindex="-1" @click.stop="tick(entry, index, $event)" />
          <span class="entryname"><span class="glyph" :class="KIND[kind(entry)] ?? 'k-doc'"></span><span class="nm">{{ entry.name }}</span></span>
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
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius-lg);
  background: var(--panel);
  box-shadow: var(--lift-panel);
  overflow: hidden;
  container-type: inline-size;
}
.side.lit { border-color: var(--disabled); }

.locline { display: flex; gap: var(--s2); padding: var(--s2); align-items: center; }
.sift { display: flex; gap: var(--s2); align-items: center; padding: 0 var(--s2) var(--s2); }
.sift input { flex: 1; min-width: 0; }
.sift-n { font-size: var(--fine); color: var(--text-faint); white-space: nowrap; }
.up {
  font: inherit;
  font-size: var(--small);
  color: var(--text-quiet);
  background: var(--panel);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius);
  width: 28px;
  height: 28px;
}
.jump, .up { cursor: pointer; flex: none; }
.up:disabled { opacity: 0.4; cursor: default; }
/* Not the global `.path` word-break: this one ellipsizes on a single line. */
.where-input { flex: 1; min-width: 0; word-break: normal; text-overflow: ellipsis; }
.where-input:focus { color: var(--text); border-color: var(--text-faint); }

.cols, .line {
  display: grid;
  grid-template-columns: 22px minmax(0, 1fr) 64px 66px 92px;
  gap: var(--s2);
  align-items: center;
  padding: 0 var(--s2);
}
:global([data-theme="retro"] .cols) {
  background: var(--surface-raised);
  color: var(--text);
  font-weight: 700;
  font-family: var(--font-mono);
}
:global([data-theme="retro"] .line) { border-bottom: var(--bw) solid var(--edge); }
.cols {
  height: 26px;
  border-bottom: var(--bw) solid var(--edge);
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
.line { height: 28px; cursor: default; font-size: var(--small); }
.line:hover { background: var(--surface-hover); }
.line.on { background: var(--surface-raised); }
.entryname { display: flex; align-items: center; gap: var(--s2); min-width: 0; }
/* The ellipsis has to be on the text, not on the flex row around it: a flex
   container clips its children and never shows the dots. */
.nm { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
/* A 12pt tile in the colour of the file's kind. */
.glyph { width: 12px; height: 12px; border-radius: 3px; flex: none; opacity: 0.9; }
.k-video { background: var(--kind-video); }
.k-image { background: var(--kind-image); }
.k-audio { background: var(--kind-audio); }
.k-archive { background: var(--kind-archive); }
.k-doc { background: var(--kind-doc); }
.k-folder { background: var(--kind-folder); border-radius: 2px 5px 3px 3px; }
.what, .line .num { color: var(--text-faint); font-size: var(--fine); white-space: nowrap; }
/* A narrow pane drops columns rather than starving the name: Type first,
   then Modified. At 860pt, the window's smallest size, a pane is about 310. */
@container (max-width: 460px) {
  .cols, .line { grid-template-columns: 22px minmax(0, 1fr) 66px 92px; }
  .cols > :nth-child(3), .line > .what { display: none; }
}
@container (max-width: 330px) {
  .cols, .line { grid-template-columns: 22px minmax(0, 1fr) 66px; }
  .cols > :nth-child(5), .line > :last-child { display: none; }
}
.nothing { font-size: var(--small); color: var(--text-faint); padding: var(--s4) var(--s2); margin: 0; }
</style>
