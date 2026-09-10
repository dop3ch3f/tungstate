<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { api, on, bytes, type Summary, type ConflictAsk } from "../api";

const props = defineProps<{ link: string | null }>();

interface Row { path: string; size: number; state: string; detail: string | null }

const rows = ref<Row[]>([]);
const live = ref<string | null>(null);
const summary = ref<Summary | null>(null);
const error = ref("");
const conflict = ref<ConflictAsk | null>(null);
const applyToAll = ref(false);
const stopping = ref(false);

// Counted here rather than waiting for the summary, so the number the user
// cares about most moves while they watch.
const reclaimed = computed(() =>
  rows.value.filter((r) => r.state === "transferred").reduce((n, r) => n + r.size, 0),
);
const done = computed(() => rows.value.filter((r) => r.state !== "live").length);

const unlisten: Array<() => void> = [];

onMounted(async () => {
  unlisten.push(
    await on.started((e) => {
      live.value = e.path;
      rows.value.unshift({ path: e.path, size: e.size, state: "live", detail: null });
    }),
    await on.finished((e) => {
      const row = rows.value.find((r) => r.path === e.path && r.state === "live");
      if (row) { row.state = e.outcome; row.detail = e.detail; }
      if (live.value === e.path) live.value = null;
    }),
    await on.conflict((e) => { conflict.value = e; }),
    await on.done((e) => { summary.value = e; live.value = null; stopping.value = false; }),
    await on.failed((e) => { error.value = e; stopping.value = false; }),
  );
});

onUnmounted(() => unlisten.forEach((f) => f()));

async function answer(action: string) {
  const all = applyToAll.value;
  conflict.value = null;
  applyToAll.value = false;
  try { await api.resolve(action, all); }
  catch (e) { error.value = String(e); }
}

async function stop() {
  stopping.value = true;
  try { await api.cancel(); } catch (e) { error.value = String(e); }
}
</script>

<template>
  <header>
    <h1>{{ props.link ?? "Transfer" }}</h1>
    <button v-if="live || (!summary && rows.length)" class="act danger" :disabled="stopping" @click="stop">
      {{ stopping ? "Finishing this file" : "Stop after this file" }}
    </button>
  </header>

  <div v-if="!props.link" class="empty">
    <strong>Nothing running.</strong>
    Choose a folder pair and press Run to start moving files.
  </div>

  <template v-else>
    <div class="readout">
      <div>
        <div class="mass">{{ bytes(reclaimed) }}</div>
        <div class="unit">freed from this machine</div>
      </div>
      <div class="count">{{ done }} of {{ rows.length }} files</div>
    </div>

    <div v-if="error" class="notice bad">{{ error }}</div>

    <div v-if="summary" class="notice calm">
      {{ summary.cancelled ? "Stopped early." : "Finished." }}
      {{ summary.transferred }} moved, {{ summary.already_present }} already there,
      {{ summary.skipped }} left for later, {{ summary.quarantined }} set aside,
      {{ summary.failed }} failed.
      <template v-if="summary.recovered">
        Picked up {{ summary.recovered }} transfer(s) interrupted last time.
      </template>
    </div>

    <div v-if="summary?.failures.length" class="notice bad">
      These were left on this machine, untouched:
      <div v-for="f in summary.failures" :key="f.path" style="margin-top:6px">
        <span class="path" style="font-family:var(--mono)">{{ f.path }}</span> — {{ f.reason }}
      </div>
    </div>

    <table v-if="rows.length" class="ledger">
      <thead>
        <tr><th>File</th><th style="text-align:right">Size</th><th>State</th></tr>
      </thead>
      <tbody>
        <tr v-for="row in rows" :key="row.path">
          <td class="path">
            {{ row.path }}
            <div v-if="row.detail" class="detail">{{ row.detail }}</div>
          </td>
          <td class="size">{{ bytes(row.size) }}</td>
          <td class="state" :class="row.state">
            {{ row.state === "live" ? "copying"
              : row.state === "transferred" ? "verified"
              : row.state === "already-present" ? "already there"
              : row.state === "quarantined" ? "set aside"
              : row.state === "skipped" ? "left for later"
              : "failed" }}
          </td>
        </tr>
      </tbody>
    </table>

    <div v-else-if="!summary" class="empty">Looking through the folder…</div>
  </template>

  <div v-if="conflict" class="veil">
    <div class="dialog" role="dialog" aria-modal="true">
      <h2>A file with this name is already there</h2>
      <p class="lede" style="margin:0">
        The contents differ, so one of them is not the file you are moving.
      </p>
      <div class="subject">{{ conflict.path }}</div>
      <div class="compare">
        <div><span>Moving</span><b>{{ bytes(conflict.incoming_size) }}</b></div>
        <div><span>Already there</span><b>{{ bytes(conflict.existing_size) }}</b></div>
      </div>
      <div class="choices">
        <button class="act primary" @click="answer('quarantine')">Set the new one aside</button>
        <button class="act" @click="answer('rename')">Keep both</button>
        <button class="act" @click="answer('skip')">Leave it on this machine</button>
        <button class="act danger" @click="answer('replace')">Replace</button>
      </div>
      <label class="forall">
        <input type="checkbox" v-model="applyToAll" />
        Do this for every other clash in this run
      </label>
    </div>
  </div>
</template>
