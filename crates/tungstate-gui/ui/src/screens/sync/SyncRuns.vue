<!-- A sync's past runs, with a way to put each one back. Copies and removals
     are separate columns: a run that copied ten and set ten aside has not
     "changed twenty files". -->
<script setup lang="ts">
import { toRef } from "vue";
import type { PastSync } from "../../engine/types";
import { files, pastCopied, pastRenamed, pastTookOff } from "../../lib/counts";
import { bytes, when } from "../../lib/format";
import { useTable } from "../../lib/table";
import Button from "../../ui/Button.vue";
import Pixels from "../../ui/Pixels.vue";
import SortHead from "../../ui/SortHead.vue";
import TableTools from "../../ui/TableTools.vue";

const props = defineProps<{
  title: string;
  runs: PastSync[];
  busy?: number | null;
  /** On one sync's own page every row is that sync, so the column and the
   *  filter by it say nothing. */
  single?: boolean;
}>();
const emit = defineEmits<{ putBack: [run: PastSync] }>();

const state = (run: PastSync) =>
  run.undone ? "was put back" : run.undoable ? "can be put back" : "deleted outright";

const table = useTable(toRef(props, "runs"), {
  columns: [
    { key: "sync", value: (r) => r.sync },
    { key: "copied", value: (r) => r.copied },
    { key: "off", value: (r) => r.taken_off },
    { key: "when", value: (r) => r.applied_at },
  ],
  text: (r) => r.sync,
  facets: [{ key: "state", label: "Show", of: state }],
  sort: { key: "when", dir: -1 },
});
</script>

<template>
  <section class="sr">
    <h2 class="sr-title">{{ props.title }}</h2>
    <div class="sr-body">
      <div v-if="!props.runs.length" class="sr-none">
        <Pixels of="no-history" :size="28" class="sr-none-art" />
        Runs are listed here, and each one can be put back on every folder.
      </div>
      <template v-else>
        <TableTools v-if="!props.single && props.runs.length > 6" :table="table" placeholder="Filter by sync" />
        <div class="sr-head">
          <span></span>
          <SortHead :table="table" column="sync">{{ props.single ? "Run" : "Sync" }}</SortHead>
          <SortHead :table="table" column="copied" numeric>Copied</SortHead>
          <SortHead :table="table" column="off" numeric>Removed</SortHead>
          <SortHead :table="table" column="when" numeric>When</SortHead>
          <span></span>
        </div>
        <ul class="sr-rows">
          <li v-for="run in table.shown.value" :key="run.plan" class="sr-row" :class="{ 'sr-back': run.undone }">
            <span class="sr-dot"></span>
            <span class="sr-what">
              <b>{{ props.single ? `Run ${run.plan}` : run.sync }}</b>
              <span class="sr-sub" v-if="run.renamed">{{ files(pastRenamed(run)) }} renamed</span>
            </span>
            <span class="num">{{ files(pastCopied(run)) }} · {{ bytes(run.bytes) }}</span>
            <span class="num">{{ files(pastTookOff(run)) }}</span>
            <span class="sr-when">{{ when(run.applied_at) }}</span>
            <span class="sr-act">
              <Button v-if="run.undoable" look="link" :busy="props.busy === run.plan" @click="emit('putBack', run)">
                {{ props.busy === run.plan ? "Putting back…" : "Put back" }}
              </Button>
              <span v-else class="sr-state">{{ state(run) }}</span>
            </span>
          </li>
        </ul>
        <p class="sr-nomatch" v-if="!table.shown.value.length">Nothing here matches that.</p>
      </template>
    </div>
  </section>
</template>

<style scoped>
/* Flat under a plain heading: a table inside a section's window is not a
   window of its own. */
.sr { display: flex; flex-direction: column; gap: var(--s2); }
.sr-title { font-size: var(--body); font-weight: 700; margin: 0; }
.sr-body { padding: 0; }
.sr-none { display: flex; align-items: center; gap: var(--s3); font-size: var(--small); color: var(--text-quiet); }
.sr-none-art { color: var(--text-faint); }
.sr-head, .sr-row {
  display: grid;
  grid-template-columns: 8px minmax(0, 1fr) 150px 90px 112px 92px;
  gap: var(--s3);
  align-items: center;
}
.sr-head { font-size: var(--fine); color: var(--text-faint); padding: 5px var(--s2); border-bottom: 1px solid var(--rule); }
.sr-head > :nth-child(n + 3) { justify-self: end; }
.sr-rows { list-style: none; margin: 0; padding: 0; }
.sr-row { padding: 8px var(--s2); border-bottom: 1px solid var(--rule); font-size: var(--small); }
.sr-row > :nth-child(n + 3) { justify-self: end; text-align: right; }
.sr-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--ok); }
.sr-back .sr-dot { background: var(--text-faint); }
.sr-back { color: var(--text-quiet); }
.sr-what { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
.sr-sub { color: var(--text-faint); font-size: var(--fine); }
.sr-when { color: var(--text-faint); font-size: var(--fine); }
.sr-act { white-space: nowrap; }
/* A status in words beside the actions column's links; never underlined, so
   it cannot be read as a link that is switched off. */
.sr-state { color: var(--text-faint); font-size: var(--fine); font-style: italic; }
.sr-nomatch { font-size: var(--small); color: var(--text-quiet); margin: var(--s3) 0 0; }
:global([data-theme="retro"] .sr-head) { background: var(--surface-raised); color: var(--text); font-weight: 700; font-family: var(--font-mono); }
</style>
