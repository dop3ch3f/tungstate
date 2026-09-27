<!-- One transfer's files, each with its state. Every engine string reaches a
     class through `lib/tone.ts`, never directly: bind `row.state` to `class`
     and a new outcome renders with no rule at all. -->
<script setup lang="ts">
import { computed, toRef } from "vue";
import { useTable } from "../../lib/table";
import { bytes } from "../../lib/format";
import { inView } from "../../lib/inview";
import { toneOfOutcome } from "../../lib/tone";
import type { Row } from "../../state/useTransfer";
import SortHead from "../../ui/SortHead.vue";
import TableTools from "../../ui/TableTools.vue";

const props = defineProps<{ rows: Row[] }>();

/** Plain words for an engine outcome. A word this map does not know is shown
 *  as itself rather than silently blanked. */
const WORD: Record<string, string> = {
  waiting: "waiting",
  live: "copying",
  checking: "checking it arrived",
  transferred: "verified",
  already_present: "already there",
  "already-present": "already there",
  quarantined: "set aside",
  skipped: "left here",
  failed: "failed",
};
const word = (state: string) => WORD[state] ?? state;

// A lookup, not an interpolated class: `check-css.mjs` cannot follow a
// computed name, and an unfollowable one is how a state ends up undrawn.
const DOT = {
  plain: "t-plain",
  live: "t-live",
  ok: "t-ok",
  hold: "t-hold",
  bad: "t-bad",
} as const;
const dot = (state: string) => DOT[toneOfOutcome(state)];

// Arrival order until a header is clicked: a live run reads top to bottom.
const table = useTable(toRef(props, "rows"), {
  columns: [
    { key: "path", value: (r) => r.path },
    { key: "state", value: (r) => word(r.state) },
    { key: "size", value: (r) => r.size },
  ],
  text: (r) => r.path,
  facets: [{ key: "state", label: "State", of: (r) => word(r.state) }],
  sort: { key: "", dir: 1 },
});

const busy = (r: Row) => r.state === "waiting" || r.state === "live" || r.state === "checking";
// Followed while in arrival order: a sort or a filter is someone looking for
// something, and should start at the top of what they asked for.
const view = computed(() =>
  inView(table.shown.value, busy, !table.narrowed.value && table.sort.value.key === ""),
);
</script>

<template>
  <TableTools :table="table" placeholder="Filter by name" />
  <div class="lg-sorts">
    <span>Sort by</span>
    <SortHead :table="table" column="path">name</SortHead>
    <SortHead :table="table" column="state">state</SortHead>
    <SortHead :table="table" column="size" numeric>size</SortHead>
  </div>
  <div>
    <p class="lg-nomatch" v-if="!table.shown.value.length">Nothing here matches that.</p>
    <!-- An exchange can list one name twice, once per direction. -->
    <div class="lg-line" v-for="(row, i) in view.rows" :key="`${view.from + i}:${row.path}`">
      <span class="lg-dot" :class="dot(row.state)"></span>
      <span class="path lg-file">{{ row.path }}</span>
      <span class="lg-word">{{ word(row.state) }}</span>
      <span class="lg-detail" v-if="row.detail">{{ row.detail }}</span>
      <span class="num lg-size">{{ bytes(row.size) }}</span>
      <span class="lg-bar" v-if="row.state === 'live' || row.state === 'checking'">
        <i :style="{ width: (row.size ? (row.done / row.size) * 100 : 0) + '%' }"></i>
      </span>
    </div>
  </div>
  <p class="lg-more" v-if="view.rows.length < table.shown.value.length">
    Showing {{ view.from + 1 }} to {{ view.from + view.rows.length }} of {{ table.shown.value.length }}.
    Filter or sort to find the rest.
  </p>
</template>

<style scoped>
.lg-sorts { display: flex; gap: var(--s3); align-items: baseline; font-size: var(--fine); color: var(--text-faint); margin: 0 0 var(--s2); }
.lg-sorts :deep(button) { width: auto; text-decoration: underline; text-underline-offset: 2px; }
.lg-nomatch { font-size: var(--small); color: var(--text-quiet); }
.lg-more { font-size: var(--fine); color: var(--text-faint); margin: var(--s2) 0 0; }
.lg-line {
  display: grid;
  grid-template-columns: 9px minmax(0, 1fr) 118px 88px;
  gap: var(--s2);
  align-items: center;
  padding: 5px 0;
  font-size: var(--fine);
  border-bottom: var(--bw) solid var(--rule);
}
.lg-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--text-faint); }
.t-plain { background: var(--text-faint); }
.t-live { background: var(--accent); }
.t-ok { background: var(--ok); }
.t-hold { background: var(--hold); }
.t-bad { background: var(--bad); }
.lg-file { color: var(--text-quiet); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; word-break: normal; }
.lg-word { color: var(--text-faint); }
.lg-detail { grid-column: 2 / -1; color: var(--text-faint); }
.lg-size { text-align: right; color: var(--text-faint); }
.lg-bar { grid-column: 1 / -1; height: 2px; background: var(--rule); border-radius: 1px; overflow: hidden; }
.lg-bar i { display: block; height: 100%; background: var(--accent); }
</style>
