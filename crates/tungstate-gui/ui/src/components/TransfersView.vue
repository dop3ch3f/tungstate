<script setup lang="ts">
import { computed } from "vue";
import { bytes, type Summary } from "../api";

export interface Row { path: string; size: number; state: string; detail: string | null }

const props = defineProps<{
  rows: Row[];
  summary: Summary | null;
  error: string;
  live: boolean;
  stopping: boolean;
}>();
const emit = defineEmits<{ stop: [] }>();

const reclaimed = computed(() =>
  props.rows.filter((r) => r.state === "transferred").reduce((n, r) => n + r.size, 0),
);
const done = computed(() => props.rows.filter((r) => r.state !== "live").length);
const progress = computed(() => (props.rows.length ? (done.value / props.rows.length) * 100 : 0));

const word: Record<string, string> = {
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
        {{ props.stopping ? "Finishing this file" : "Stop after this file" }}
      </button>
    </div>

    <div v-if="!props.rows.length && !props.summary" class="sub">
      Nothing running. Select files in the browser and move or copy them.
    </div>

    <template v-else>
      <div class="readout">
        <div>
          <div class="mass">{{ bytes(reclaimed) }}</div>
          <div class="unit">{{ props.summary?.cancelled ? "freed before stopping" : "freed from this machine" }}</div>
        </div>
        <div class="count">{{ done }} of {{ props.rows.length }} files</div>
      </div>
      <div class="bar"><div :style="{ width: progress + '%' }"></div></div>

      <div v-if="props.error" class="notice bad">{{ props.error }}</div>

      <div v-if="props.summary" class="notice" :class="props.summary.failed ? 'bad' : 'good'">
        {{ props.summary.cancelled ? "Stopped early." : "Finished." }}
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
            <td class="state" :class="row.state">{{ word[row.state] ?? row.state }}</td>
          </tr>
        </tbody>
      </table>
    </template>
  </div>
</template>
