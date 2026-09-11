<script setup lang="ts">
import { computed } from "vue";
import { bytes, type InterruptedRun, type Summary } from "../api";

export interface Row {
  path: string;
  size: number;
  state: string;
  detail: string | null;
  /// Bytes transferred so far, for the file in flight.
  done: number;
}

const props = defineProps<{
  rows: Row[];
  summary: Summary | null;
  error: string;
  live: boolean;
  stopping: boolean;
  interrupted: InterruptedRun[];
  busy: string;
  note: string;
}>();
const emit = defineEmits<{ stop: []; resume: [link: string]; discard: [link: string] }>();

const reclaimed = computed(() =>
  props.rows.filter((r) => r.state === "transferred").reduce((n, r) => n + r.size, 0),
);
const settled = computed(() =>
  props.rows.filter((r) => r.state !== "live" && r.state !== "waiting").length,
);
// Bytes rather than file count: fifteen small files and one enormous one
// would otherwise sit at 94% for most of the run.
const totalBytes = computed(() => props.rows.reduce((n, r) => n + r.size, 0));
const movedBytes = computed(() =>
  props.rows.reduce((n, r) => n + (r.state === "waiting" ? 0 : r.state === "live" ? r.done : r.size), 0),
);
const progress = computed(() => (totalBytes.value ? (movedBytes.value / totalBytes.value) * 100 : 0));

const word: Record<string, string> = {
  waiting: "waiting",
  live: "copying",
  transferred: "verified",
  "already-present": "already there",
  quarantined: "set aside",
  skipped: "left here",
  failed: "failed",
};
</script>

<template>
  <div class="sheet">
    <div style="display:flex; align-items:baseline; justify-content:space-between; gap:12px">
      <h1>Transfers</h1>
      <button v-if="props.live" class="btn danger" :disabled="props.stopping" @click="emit('stop')">
        {{ props.stopping ? "Finishing what is in flight" : "Stop after these files" }}
      </button>
    </div>

    <!-- Outside the branches below on purpose. A run that fails before its
         first file has no rows and no summary, and that is exactly when the
         reason is the only thing worth showing. -->
    <div v-if="props.note" class="notice good">{{ props.note }}</div>

    <div v-if="props.error" class="notice bad">
      <b>That transfer could not start.</b>
      <div style="font-family: var(--mono); margin-top: 6px">{{ props.error }}</div>
      <div class="note" style="margin-top: 6px">Nothing was moved, and every original is where it was.</div>
    </div>

    <!-- Work that outlived the process. Offered, never acted on: resuming
         starts a multi-gigabyte transfer and discarding deletes something,
         and neither is a thing to do to somebody while they are reading. -->
    <div v-for="run in props.interrupted" :key="run.link" class="stranded">
      <div>
        <strong>{{ run.files }} {{ run.files === 1 ? "file was" : "files were" }} left
        unfinished</strong>
        — {{ bytes(run.bytes) }} towards <span class="where">{{ run.destination }}</span>.
        <div class="detail">{{ run.names.join(", ") }}</div>
        <div class="note">
          Nothing was lost: every original is still where it was. Resuming copies
          the unfinished ones again from the start. Cleaning up removes what was
          part-copied at the destination and leaves the originals alone.
        </div>
      </div>
      <div class="go">
        <button class="btn primary" :disabled="!!props.busy || props.live"
                @click="emit('resume', run.link)">
          {{ props.busy === run.link ? "Working…" : "Resume" }}
        </button>
        <button class="btn" :disabled="!!props.busy || props.live"
                @click="emit('discard', run.link)">Clean up</button>
      </div>
    </div>

    <div v-if="!props.rows.length && !props.summary && !props.error
                && !props.interrupted.length && !props.note"
         class="sub">
      Nothing running. Select files in the browser and move or copy them.
    </div>

    <template v-else-if="props.rows.length || props.summary">
      <div class="readout">
        <div>
          <div class="mass">{{ bytes(reclaimed) }}</div>
          <div class="unit">{{ props.summary?.cancelled ? "freed before stopping" : "freed from this machine" }}</div>
        </div>
        <div class="count">{{ settled }} of {{ props.rows.length }} files</div>
      </div>
      <div class="bar"><div :style="{ width: progress + '%' }"></div></div>

      <div v-if="props.summary?.destination_lost" class="notice bad">
        <b>Stopped: the destination is no longer reachable.</b>
        It may have been disconnected, or something else is now at that location.
        Nothing further was moved and every remaining original is untouched here.
        Reconnect it and run this again.
      </div>

      <div v-if="props.summary" class="notice" :class="props.summary.failed ? 'bad' : 'good'">
        {{ props.summary.cancelled ? "Stopped early." : props.summary.destination_lost ? "Stopped." : "Finished." }}
        {{ props.summary.transferred }} moved,
        {{ props.summary.already_present }} already there,
        {{ props.summary.skipped }} left here,
        {{ props.summary.quarantined }} set aside,
        {{ props.summary.failed }} failed.
        <template v-if="props.summary.recovered">
          Picked up {{ props.summary.recovered }} interrupted transfer(s) from last time.
        </template>
      </div>

      <div v-if="props.summary?.failures.length" class="notice bad">
        These stayed on this machine, untouched:
        <div v-for="f in props.summary.failures" :key="f.path" style="margin-top:5px">
          <span style="font-family: var(--mono)">{{ f.path }}</span> — {{ f.reason }}
        </div>
      </div>

      <table class="ledger">
        <thead><tr><th>File</th><th class="num">Size</th><th>State</th></tr></thead>
        <tbody>
          <tr v-for="row in props.rows" :key="row.path">
            <td class="path">
              {{ row.path }}
              <div v-if="row.detail" class="detail">{{ row.detail }}</div>
            </td>
            <td class="num">{{ bytes(row.size) }}</td>
            <td class="state" :class="row.state">
              {{ word[row.state] ?? row.state }}
              <!-- Only the file in flight gets a bar; a row that is waiting or
                   finished says so in a word and needs no decoration. -->
              <span v-if="row.state === 'live' && row.size" class="filebar">
                <span :style="{ width: Math.min(100, (row.done / row.size) * 100) + '%' }"></span>
              </span>
              <span v-if="row.state === 'live' && row.size" class="pct">
                {{ bytes(row.done) }} of {{ bytes(row.size) }}
              </span>
            </td>
          </tr>
        </tbody>
      </table>
    </template>
  </div>
</template>
