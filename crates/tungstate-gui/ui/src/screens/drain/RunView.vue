<!-- The transfer queue: the one running with its files, the ones waiting
     behind it, and the ones that have finished, each in its own row. -->
<script setup lang="ts">
import { computed, ref } from "vue";
import { useTransfer, type Ran } from "../../state/useTransfer";
import { bytes, ordinal, shortPath } from "../../lib/format";
import { files, inLine, sent } from "../../lib/counts";
import type { JobView } from "../../engine/types";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import StopPair from "../../ui/StopPair.vue";
import Empty from "../../ui/Empty.vue";
import Ledger from "./Ledger.vue";

const t = useTransfer();

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
/** From the job event, or from the queue if this window opened mid-run. */
const now = computed(() => t.current.value ?? t.queue.value.running);
const waiting = computed(() => t.queue.value.waiting);

const route = (job: JobView) =>
  job.source
    ? `${shortPath(job.source)} ${job.exchange ? "⇄" : "→"} ${shortPath(job.destination)}`
    : "A transfer";
const verb = (job: JobView) => (job.exchange ? "Exchange" : job.removes_originals ? "Move" : "Copy");
const size = (job: JobView) =>
  job.files === null ? "counting…" : `${files(inLine({ ...job, files: job.files }))}, ${bytes(job.bytes ?? 0)}`;

/** One line for how a transfer ended. The full account is in its notice. */
function outcome(run: Ran): string {
  const s = run.summary;
  if (!s) return "Could not carry on";
  if (s.destination_lost) return "The far side disappeared";
  if (s.cancelled) return "Stopped";
  const done = `${run.job.removes_originals ? "Moved" : "Copied"} ${files(sent(s))}, ${bytes(s.bytes)}`;
  return s.failed ? `${done}. ${s.failed} failed` : done;
}
const troubled = (run: Ran) => !run.summary || run.summary.failed > 0 || run.summary.destination_lost;

/** Which finished transfer is open. Unless one was chosen, the newest is,
 *  while nothing else is running: it is the one just watched. */
const chosen = ref<number | null>(null);
const open = computed(() =>
  chosen.value ?? (t.running.value ? null : t.finished.value[0]?.job.id ?? null),
);
const toggle = (id: number) => (chosen.value = open.value === id ? -1 : id);
</script>

<template>
  <div class="run-wrap">
    <Notice tone="bad" v-if="t.deaf.value">{{ t.deaf.value }}</Notice>
    <Notice tone="bad" v-if="t.problem.value">{{ t.problem.value }}</Notice>
    <Notice tone="plain" v-if="t.cleaned.value">
      Cleaned up {{ bytes(Number(t.cleaned.value)) }} of part-copied files. The originals are untouched.
    </Notice>

    <Empty
      v-if="!t.running.value && !waiting.length && !t.finished.value.length"
      art="no-runs"
      line="Nothing running. Tick some files in the browser and press Copy or Move."
    />

    <section class="run-now" v-if="now">
      <div class="run-head">
        <h2 class="path run-route">{{ route(now) }}</h2>
        <span class="run-verb">{{ verb(now) }}</span>
        <span class="run-spacer"></span>
        <StopPair :stopping="t.stopping.value" :halting="t.halting.value" @stop="t.cancel()" @now="t.stopNow()" />
      </div>
      <div class="run-readout" v-if="planned">
        <span class="run-mass num">{{ bytes(moved) }}</span>
        <span class="run-of">moved</span>
        <span class="run-of">
          <b class="num">{{ t.settled.value }}</b> of <b class="num">{{ planned }}</b> files
        </span>
        <span class="run-at" v-if="t.atOnce.value">{{ t.atOnce.value }} at a time</span>
      </div>
      <p class="run-quiet" v-else>Working out what to send…</p>
      <Ledger v-if="planned" :rows="t.rows.value" />
    </section>

    <section class="run-list" v-if="waiting.length">
      <div class="run-list-head">
        <h3>Waiting</h3>
        <span class="run-quiet">Stopping the transfer above clears these too.</span>
      </div>
      <div class="run-row" v-for="(job, i) in waiting" :key="job.id">
        <span class="run-place num">{{ ordinal(i + 2) }}</span>
        <span class="path run-row-route">{{ route(job) }}</span>
        <span class="run-row-what">{{ verb(job) }} · {{ size(job) }}</span>
        <Button look="link" :busy="t.unqueuing(job.id)" @click="t.unqueue(job.id)">Remove</Button>
      </div>
    </section>

    <section class="run-list" v-if="t.finished.value.length">
      <div class="run-list-head"><h3>Recently finished</h3></div>
      <template v-for="run in t.finished.value" :key="run.job.id">
        <div class="run-row">
          <span class="run-dot" :class="troubled(run) ? 't-bad' : 't-ok'"></span>
          <span class="path run-row-route">{{ route(run.job) }}</span>
          <span class="run-row-what">{{ outcome(run) }}</span>
          <Button look="link" @click="toggle(run.job.id)">{{ open === run.job.id ? "Hide" : "Details" }}</Button>
        </div>
        <div class="run-open" v-if="open === run.job.id">
          <Notice tone="bad" v-if="run.problem">{{ run.problem }}</Notice>
          <Notice v-if="run.summary" :tone="run.summary.failed || run.summary.destination_lost ? 'bad' : 'plain'">
            <template v-if="run.summary.destination_lost">
              The far side disappeared part-way through. Nothing was deleted that had not arrived.
            </template>
            <template v-else-if="run.summary.cancelled">Stopped when you asked.</template>
            <template v-else>Finished.</template>
            {{ run.job.removes_originals ? "Moved" : "Copied" }} {{ files(sent(run.summary)) }}, {{ bytes(run.summary.bytes) }}.
            <template v-if="run.summary.already_present">
              {{ run.summary.already_present }} were already there.
            </template>
            <template v-if="run.summary.quarantined">
              {{ run.summary.quarantined }} set aside for you.
            </template>
            <template v-if="run.summary.skipped">
              {{ run.summary.skipped }} left here.
            </template>
            <template v-if="run.summary.failed">
              {{ run.summary.failed }} could not be moved; their originals are untouched.
            </template>
            <template v-if="run.summary.pruned">
              {{ run.summary.pruned }} source director{{ run.summary.pruned === 1 ? "y" : "ies" }}
              removed, because the move emptied {{ run.summary.pruned === 1 ? "it" : "them" }}.
            </template>
            <template v-if="run.summary.recovered">
              {{ run.summary.recovered }} unfinished operation{{ run.summary.recovered === 1 ? "" : "s" }}
              from an earlier run {{ run.summary.recovered === 1 ? "was" : "were" }} cleaned up first.
            </template>
          </Notice>
          <ul class="run-failures" v-if="run.summary?.failures.length">
            <li v-for="fail in run.summary.failures" :key="fail.path">
              <span class="path">{{ fail.path }}</span>
              <span class="run-why">{{ fail.reason }}</span>
            </li>
          </ul>
          <Ledger v-if="run.rows.length" :rows="run.rows" />
        </div>
      </template>
    </section>
  </div>
</template>

<style scoped>
.run-wrap { display: flex; flex-direction: column; gap: var(--s5); min-height: 0; overflow-y: auto; }

.run-now { display: flex; flex-direction: column; gap: var(--s3); }
.run-head { display: flex; align-items: center; gap: var(--s3); min-width: 0; }
.run-route { font-size: var(--body); font-weight: 600; margin: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.run-verb { font-size: var(--small); color: var(--text-quiet); }
.run-spacer { flex: 1; }
.run-readout { display: flex; align-items: baseline; gap: var(--s5); }
.run-mass { font-size: 30px; font-weight: 500; letter-spacing: -0.02em; }
.run-of { font-size: var(--small); color: var(--text-quiet); }
.run-of b { color: var(--text); font-weight: 600; }
.run-at { font-size: var(--fine); color: var(--text-faint); }
.run-quiet { font-size: var(--fine); color: var(--text-faint); margin: 0; }

.run-list { display: flex; flex-direction: column; }
.run-list-head { display: flex; align-items: baseline; gap: var(--s3); margin-bottom: var(--s2); }
.run-list-head h3 { font-size: var(--small); font-weight: 600; margin: 0; }
.run-row {
  display: grid;
  grid-template-columns: 36px minmax(0, 1fr) auto auto;
  gap: var(--s3);
  align-items: center;
  padding: var(--s2) 0;
  font-size: var(--small);
  border-bottom: var(--bw) solid var(--rule);
}
.run-place { color: var(--text-faint); font-size: var(--fine); }
.run-row-route { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; word-break: normal; }
.run-row-what { color: var(--text-quiet); }
.run-dot { width: 7px; height: 7px; border-radius: 50%; justify-self: center; }
.t-ok { background: var(--ok); }
.t-bad { background: var(--bad); }
.run-open { display: flex; flex-direction: column; gap: var(--s2); padding: var(--s3) 0 var(--s4); }

.run-failures { list-style: none; margin: 0; padding: 0; font-size: var(--fine); }
.run-failures li { display: flex; gap: var(--s3); padding: 3px 0; color: var(--text-quiet); }
.run-why { color: var(--bad); margin-left: auto; }
</style>
