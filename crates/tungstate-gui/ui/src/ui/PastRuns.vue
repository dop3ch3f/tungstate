<!-- A section's own history: every tidy, or every clean-up, with a way to put
     each one back. History has everything; this has the part that belongs
     here, where somebody is standing when they want to undo it. -->
<script setup lang="ts">
import { toRef } from "vue";
import type { PastRun } from "../engine/types";
import type { Section } from "../lib/icons";
import { files, ran } from "../lib/counts";
import { bytes, shortPath, when } from "../lib/format";
import { useTable } from "../lib/table";
import Button from "./Button.vue";
import Pixels from "./Pixels.vue";
import SortHead from "./SortHead.vue";
import TableTools from "./TableTools.vue";
import TitleBar from "./TitleBar.vue";

const props = defineProps<{
  of: Section;
  title: string;
  /** What an empty list says, which is also the promise the list makes. */
  empty: string;
  /** "Filed" or "Set aside": what a run did to its files. */
  verb: string;
  runs: PastRun[];
  /** Which run is being put back right now, so its button says so. */
  busy?: number | null;
}>();
const emit = defineEmits<{ putBack: [run: PastRun] }>();

const inTrash = (run: PastRun) => !run.undone && !run.undoable;
const state = (run: PastRun) =>
  run.undone ? "put back" : inTrash(run) ? "in the trash" : "can be put back";

const table = useTable(toRef(props, "runs"), {
  columns: [
    { key: "name", value: (r) => r.name },
    { key: "files", value: (r) => r.files },
    { key: "bytes", value: (r) => r.bytes },
    { key: "when", value: (r) => r.applied_at },
  ],
  text: (r) => `${r.name} ${r.root}`,
  facets: [{ key: "state", label: "Show", of: state }],
  sort: { key: "when", dir: -1 },
});
</script>

<template>
  <section class="pr">
    <TitleBar :of="props.of" :title="props.title" />
    <div class="pr-body">
      <div v-if="!props.runs.length" class="pr-none">
        <Pixels of="no-history" :size="28" class="pr-none-art" />
        {{ props.empty }}
      </div>
      <template v-else>
        <TableTools :table="table" placeholder="Filter by folder" />
        <div class="pr-head">
          <span></span>
          <SortHead :table="table" column="name">Folder</SortHead>
          <SortHead :table="table" column="files" numeric>Files</SortHead>
          <SortHead :table="table" column="bytes" numeric>Size</SortHead>
          <SortHead :table="table" column="when" numeric>When</SortHead>
          <span></span>
        </div>
        <ul class="pr-rows">
          <li v-for="run in table.shown.value" :key="run.plan" class="pr-row" :class="{ 'pr-back': run.undone }">
            <span class="pr-dot"></span>
            <span class="pr-what">
              <b>{{ run.name }}</b>
              <span class="path pr-path">{{ shortPath(run.root) }}</span>
            </span>
            <span class="num">{{ inTrash(run) ? "Trashed" : props.verb }} {{ files(ran(run)) }}</span>
            <span class="num">{{ bytes(run.bytes) }}</span>
            <span class="pr-when">{{ when(run.applied_at) }}</span>
            <span class="pr-act">
              <Button v-if="run.undoable" look="link" @click="emit('putBack', run)">
                {{ props.busy === run.plan ? "Putting back…" : "Put back" }}
              </Button>
              <span v-else class="pr-state">{{ state(run) }}</span>
            </span>
          </li>
        </ul>
        <p class="pr-nomatch" v-if="!table.shown.value.length">Nothing here matches that.</p>
      </template>
    </div>
  </section>
</template>

<style scoped>
.pr {
  background: var(--panel);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius-lg);
  box-shadow: var(--lift-panel);
  overflow: hidden;
}
.pr > :first-child { padding: var(--s4) var(--s4) 0; }
.pr-body { padding: var(--s3) var(--s4) var(--s4); }
.pr-none { display: flex; align-items: center; gap: var(--s3); font-size: var(--small); color: var(--text-quiet); }
.pr-none-art { color: var(--text-faint); }
.pr-head, .pr-row {
  display: grid;
  grid-template-columns: 8px minmax(0, 1fr) 130px 76px 112px 92px;
  gap: var(--s3);
  align-items: center;
}
.pr-head { font-size: var(--fine); color: var(--text-faint); padding: 5px var(--s2); border-bottom: var(--bw) solid var(--edge); }
.pr-head > :nth-child(n + 3) { justify-self: end; }
.pr-rows { list-style: none; margin: 0; padding: 0; }
.pr-row { padding: 8px var(--s2); border-bottom: var(--bw) solid var(--edge); font-size: var(--small); }
.pr-row > :nth-child(n + 3) { justify-self: end; text-align: right; }
.pr-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--ok); }
.pr-back .pr-dot { background: var(--text-faint); }
.pr-back { color: var(--text-quiet); }
.pr-what { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
.pr-what b { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pr-path { color: var(--text-faint); font-size: var(--fine); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pr-when { color: var(--text-faint); font-size: var(--fine); }
.pr-act { white-space: nowrap; }
.pr-state { color: var(--text-faint); font-size: var(--fine); }
.pr-nomatch { font-size: var(--small); color: var(--text-quiet); margin: var(--s3) 0 0; }
/* Retro: the bar is flush with the window's edges, the header a raised band. */
:global([data-theme="retro"] .pr > :first-child) { padding: 6px 7px 6px var(--s3); }
:global([data-theme="retro"] .pr-head) { background: var(--surface-raised); color: var(--text); font-weight: 700; font-family: var(--font-mono); }
</style>
