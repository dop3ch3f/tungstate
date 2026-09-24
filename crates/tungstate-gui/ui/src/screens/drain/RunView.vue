<!-- A transfer while it happens, and what it did when it stops.
     Every engine string reaches a class through `lib/tone.ts`, never directly:
     bind `row.state` to `class` and a new outcome renders with no rule at all. -->
<script setup lang="ts">
import { computed } from "vue";
import { useTable } from "../../lib/table";
import SortHead from "../../ui/SortHead.vue";
import TableTools from "../../ui/TableTools.vue";
import { useTransfer } from "../../state/useTransfer";
import { bytes } from "../../lib/format";
import { toneOfOutcome } from "../../lib/tone";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import Empty from "../../ui/Empty.vue";

const t = useTransfer();

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
const table = useTable(t.rows, {
  columns: [
    { key: "path", value: (r) => r.path },
    { key: "state", value: (r) => word(r.state) },
    { key: "size", value: (r) => r.size },
  ],
  text: (r) => r.path,
  facets: [{ key: "state", label: "State", of: (r) => word(r.state) }],
  sort: { key: "", dir: 1 },
});

/** Bytes that have arrived and been verified. On a move this is also the space
 *  freed on this machine, which is the number the whole product exists for.
 *
 *  Not `Summary.recovered`, which is "interrupted operations cleaned up before
 *  this run started" -- a count of operations. Formatting that as bytes and
 *  calling it reclaimed produced "0 B reclaimed" at the end of a 600MB move,
 *  which is the same defect slice 7b shipped wearing different clothes. */
const moved = computed(() =>
  t.rows.value
    .filter((r) => r.state === "transferred")
    .reduce((n, r) => n + r.size, 0),
);
const planned = computed(() => t.rows.value.length);
</script>

<template>
  <div class="run-wrap">
    <Notice tone="bad" v-if="t.deaf.value">{{ t.deaf.value }}</Notice>
    <Notice tone="bad" v-if="t.problem.value">{{ t.problem.value }}</Notice>
    <Notice tone="plain" v-if="t.cleaned.value">
      Cleaned up {{ bytes(Number(t.cleaned.value)) }} of part-copied files. The originals are untouched.
    </Notice>

    <div class="run-readout" v-if="planned">
      <span class="run-mass num">{{ bytes(moved) }}</span>
      <span class="run-of">moved</span>
      <span class="run-of">
        <b class="num">{{ t.settled.value }}</b> of <b class="num">{{ planned }}</b> files
      </span>
      <span class="run-at" v-if="t.atOnce.value">{{ t.atOnce.value }} at a time</span>
      <span class="run-at" v-if="t.queued.value">{{ t.queued.value }} queued behind this</span>
      <span class="run-spacer"></span>
      <Button v-if="t.running.value && !t.stopping.value" @click="t.cancel()">Stop after these files</Button>
      <Button v-else-if="t.running.value && !t.halting.value" look="danger" @click="t.stopNow()">Stop now</Button>
    </div>

    <Empty v-if="!planned && !t.summary.value" art="no-runs" line="Nothing running. Tick some files in the browser and press Copy or Move." />

    <template v-if="planned">
      <TableTools :table="table" placeholder="Filter by name" />
      <div class="run-sorts">
        <span>Sort by</span>
        <SortHead :table="table" column="path">name</SortHead>
        <SortHead :table="table" column="state">state</SortHead>
        <SortHead :table="table" column="size" numeric>size</SortHead>
      </div>
    </template>
    <div class="run-ledger" v-if="planned">
      <p class="run-nomatch" v-if="!table.shown.value.length">Nothing here matches that.</p>
      <div class="run-line" v-for="row in table.shown.value" :key="row.path">
        <span class="run-dot" :class="dot(row.state)"></span>
        <span class="path run-file">{{ row.path }}</span>
        <span class="run-word">{{ word(row.state) }}</span>
        <span class="run-detail" v-if="row.detail">{{ row.detail }}</span>
        <span class="num run-size">{{ bytes(row.size) }}</span>
        <span class="run-bar" v-if="row.state === 'live' || row.state === 'checking'">
          <i :style="{ width: (row.size ? (row.done / row.size) * 100 : 0) + '%' }"></i>
        </span>
      </div>
    </div>

    <div class="run-summary" v-if="t.summary.value">
      <Notice :tone="t.summary.value.failed || t.summary.value.destination_lost ? 'bad' : 'plain'">
        <template v-if="t.summary.value.destination_lost">
          The far side disappeared part-way through. Nothing was deleted that had not arrived.
        </template>
        <template v-else-if="t.summary.value.cancelled">Stopped when you asked.</template>
        <template v-else>Finished.</template>
        Moved {{ t.summary.value.transferred }} files, {{ bytes(t.summary.value.bytes) }}.
        <template v-if="t.summary.value.already_present">
          {{ t.summary.value.already_present }} were already there.
        </template>
        <template v-if="t.summary.value.quarantined">
          {{ t.summary.value.quarantined }} set aside for you.
        </template>
        <template v-if="t.summary.value.skipped">
          {{ t.summary.value.skipped }} left here.
        </template>
        <template v-if="t.summary.value.failed">
          {{ t.summary.value.failed }} could not be moved; their originals are untouched.
        </template>
        <template v-if="t.summary.value.pruned">
          {{ t.summary.value.pruned }} source director{{ t.summary.value.pruned === 1 ? "y" : "ies" }}
          removed, because the move emptied {{ t.summary.value.pruned === 1 ? "it" : "them" }}.
        </template>
        <template v-if="t.summary.value.recovered">
          {{ t.summary.value.recovered }} unfinished operation{{ t.summary.value.recovered === 1 ? "" : "s" }}
          from an earlier run {{ t.summary.value.recovered === 1 ? "was" : "were" }} cleaned up first.
        </template>
      </Notice>
      <ul class="run-failures" v-if="t.summary.value.failures.length">
        <li v-for="fail in t.summary.value.failures" :key="fail.path">
          <span class="path">{{ fail.path }}</span>
          <span class="run-why">{{ fail.reason }}</span>
        </li>
      </ul>
    </div>
  </div>
</template>

<style scoped>
.run-wrap { display: flex; flex-direction: column; gap: var(--s3); min-height: 0; }

.run-readout { display: flex; align-items: baseline; gap: var(--s5); }
.run-mass { font-size: 30px; font-weight: 500; letter-spacing: -0.02em; }
.run-of { font-size: var(--small); color: var(--text-quiet); }
.run-of b { color: var(--text); font-weight: 600; }
.run-at { font-size: var(--fine); color: var(--text-faint); }
.run-spacer { flex: 1; }

.run-ledger { flex: 1; overflow-y: auto; min-height: 0; }
.run-sorts { display: flex; gap: var(--s3); align-items: baseline; font-size: var(--fine); color: var(--text-faint); margin: 0 0 var(--s2); }
.run-sorts :deep(button) { width: auto; text-decoration: underline; text-underline-offset: 2px; }
.run-nomatch { font-size: var(--small); color: var(--text-quiet); }
.run-line {
  display: grid;
  grid-template-columns: 9px minmax(0, 1fr) 118px 88px;
  gap: var(--s2);
  align-items: center;
  padding: 5px 0;
  font-size: var(--fine);
  border-bottom: var(--bw) solid var(--edge);
}
.run-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--text-faint); }
.t-plain { background: var(--text-faint); }
.t-live { background: var(--accent); }
.t-ok { background: var(--ok); }
.t-hold { background: var(--hold); }
.t-bad { background: var(--bad); }
.run-file { color: var(--text-quiet); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; word-break: normal; }
.run-word { color: var(--text-faint); }
.run-detail { grid-column: 2 / -1; color: var(--text-faint); }
.run-size { text-align: right; color: var(--text-faint); }
.run-bar { grid-column: 1 / -1; height: 2px; background: var(--edge); border-radius: 1px; overflow: hidden; }
.run-bar i { display: block; height: 100%; background: var(--accent); }

.run-summary { display: flex; flex-direction: column; gap: var(--s2); }
.run-failures { list-style: none; margin: 0; padding: 0; font-size: var(--fine); }
.run-failures li { display: flex; gap: var(--s3); padding: 3px 0; color: var(--text-quiet); }
.run-why { color: var(--bad); margin-left: auto; }
</style>
