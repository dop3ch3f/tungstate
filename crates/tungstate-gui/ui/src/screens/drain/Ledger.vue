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

/** `under` is where the run's files go: said once in the heading, so each
 *  row shows only the part of its path below it. */
const props = defineProps<{ rows: Row[]; under?: string }>();

/** Below this many files, filtering and sorting are more to read than to use. */
const FEW = 12;

const name = (path: string) => {
  const root = props.under?.replace(/\/+$/, "");
  return root && path.startsWith(`${root}/`) ? path.slice(root.length + 1) : path;
};

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
// `live` and `checking` are states, not outcomes, so the outcome map calls
// them plain; a file on the move beats, in the colour of its bar.
const dot = (state: string) =>
  state === "live" ? DOT.live : state === "checking" ? "t-checking" : DOT[toneOfOutcome(state)];

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

const moving = (r: Row) => r.state === "live" || r.state === "checking";
/** How far along a moving file is: copying counts bytes sent, checking
 *  counts bytes read back from where it went. */
const part = (r: Row) => (r.state === "checking" ? r.checked : r.done);
const percent = (r: Row) => (r.size ? Math.min(100, Math.floor((part(r) / r.size) * 100)) : 0);

const busy = (r: Row) => r.state === "waiting" || r.state === "live" || r.state === "checking";
// Followed while in arrival order: a sort or a filter is someone looking for
// something, and should start at the top of what they asked for.
const view = computed(() =>
  inView(table.shown.value, busy, !table.narrowed.value && table.sort.value.key === ""),
);
</script>

<template>
  <TableTools v-if="props.rows.length > FEW" :table="table" placeholder="Filter by name" />
  <div class="lg-sorts" v-if="props.rows.length > FEW">
    <span>Sort by</span>
    <SortHead :table="table" column="path">name</SortHead>
    <SortHead :table="table" column="state">state</SortHead>
    <SortHead :table="table" column="size" numeric>size</SortHead>
  </div>
  <div>
    <p class="lg-nomatch" v-if="!table.shown.value.length">Nothing here matches that.</p>
    <!-- An exchange can list one name twice, once per direction. -->
    <!-- Cards, not ruled lines: a file on the move is raised with a thick
         bar, so its progress cannot be mistaken for the line between rows. -->
    <ul class="lg-stack">
      <li
        v-for="(row, i) in view.rows"
        :key="`${view.from + i}:${row.path}`"
        class="lg-line"
        :class="{ 'lg-moving': moving(row) }"
      >
        <span class="lg-dot" :class="dot(row.state)"></span>
        <span class="path lg-file" :title="row.path">{{ name(row.path) }}</span>
        <span class="lg-word" v-if="!moving(row)">{{ word(row.state) }}</span>
        <span class="num lg-size">{{ bytes(row.size) }}</span>
        <span class="lg-detail" v-if="row.detail">{{ row.detail }}</span>
        <span class="lg-meter" v-if="moving(row)">
          <span class="lg-bar" :class="{ 'lg-checking': row.state === 'checking' }" role="progressbar" :aria-valuenow="percent(row)" aria-valuemin="0" aria-valuemax="100">
            <i :style="{ width: percent(row) + '%' }"></i>
          </span>
          <span class="lg-pc"><span class="lg-now">{{ word(row.state) }}</span> <b class="num">{{ percent(row) }}%</b></span>
        </span>
      </li>
    </ul>
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
.lg-stack { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--s1); }
.lg-line {
  display: grid;
  grid-template-columns: 9px minmax(0, 1fr) 128px 72px;
  gap: 2px var(--s2);
  align-items: center;
  padding: 7px var(--s3);
  font-size: var(--fine);
  background: var(--surface);
  border: var(--bw) solid transparent;
  border-radius: var(--radius);
}
/* The file on the move: raised like a window, so it is the thing looked at. */
/* A moving file has no state column: its state sits by its percent. */
.lg-moving { grid-template-columns: 9px minmax(0, 1fr) 72px; }
.lg-moving {
  background: var(--panel);
  border-color: var(--edge);
  box-shadow: var(--lift);
  padding: var(--s2) var(--s3);
  margin: 2px 0;
}
.lg-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--text-faint); }
.t-plain { background: var(--text-faint); }
.t-live { background: var(--accent); animation: lg-beat 1.2s ease-in-out infinite; }
.t-ok { background: var(--ok); }
.t-checking { background: var(--ok); animation: lg-beat 1.2s ease-in-out infinite; }
.t-hold { background: var(--hold); }
.t-bad { background: var(--bad); }
.lg-file { color: var(--text-quiet); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; word-break: normal; }
.lg-moving .lg-file { color: var(--text); font-weight: 600; }
.lg-word { color: var(--text-faint); text-align: right; }
.lg-moving .lg-word { color: var(--text-quiet); }
.lg-detail { grid-column: 2 / -1; color: var(--text-faint); }
.lg-size { text-align: right; color: var(--text-faint); white-space: nowrap; }
.lg-meter { grid-column: 2 / -1; display: flex; align-items: center; gap: var(--s3); margin-top: var(--s1); }
.lg-bar {
  flex: 1;
  height: 10px;
  background: var(--surface);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius);
  overflow: hidden;
}
/* Stripes that travel along the filled part: the file is moving even when
   the number has not changed since the last glance. */
.lg-bar i {
  display: block;
  height: 100%;
  background-color: var(--accent);
  background-image: repeating-linear-gradient(
    -45deg,
    transparent 0 6px,
    color-mix(in srgb, var(--accent-ink) 28%, transparent) 6px 12px
  );
  background-size: 17px 100%;
  animation: lg-travel 0.9s linear infinite;
  transition: width 0.3s ease-out;
}
.lg-pc { flex: none; width: 21ch; font-size: var(--small); text-align: right; white-space: nowrap; color: var(--text); }
.lg-now { font-size: var(--fine); color: var(--text-quiet); }
/* Checking reads the file back from where it went: a second pass, in the
   colour of "it arrived", so it is not mistaken for a copy going backwards. */
.lg-checking i { background-color: var(--ok); }
@keyframes lg-travel { to { background-position: 17px 0; } }
@keyframes lg-beat { 50% { transform: scale(1.5); opacity: 0.6; } }
@media (prefers-reduced-motion: reduce) {
  .lg-bar i, .t-live, .t-checking { animation: none; }
}
</style>
