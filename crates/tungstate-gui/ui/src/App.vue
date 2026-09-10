<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, useTemplateRef } from "vue";
import Pane from "./components/Pane.vue";
import SyncModal, { type Payload } from "./components/SyncModal.vue";
import TransfersView, { type Row } from "./components/TransfersView.vue";
import ActivityView from "./components/ActivityView.vue";
import Welcome from "./components/Welcome.vue";
import { api, on, bytes, type Place, type Summary, type ConflictAsk, type Link } from "./api";

type Tab = "browse" | "transfers" | "activity";

const tab = ref<Tab>("browse");
const onboarding = ref(false);
const places = ref<Place[]>([]);
const links = ref<Link[]>([]);
const error = ref("");

const left = useTemplateRef<InstanceType<typeof Pane>>("left");
const right = useTemplateRef<InstanceType<typeof Pane>>("right");

const leftStart = ref("");
const rightStart = ref("");
const activeSide = ref<"left" | "right">("left");

const picked = ref<{ left: string[]; right: string[] }>({ left: [], right: [] });
const pickedBytes = ref<{ left: number; right: number }>({ left: 0, right: 0 });
const paths = ref<{ left: string; right: string }>({ left: "", right: "" });

const source = computed(() => (activeSide.value === "left" ? "left" : "right") as "left" | "right");
const target = computed(() => (activeSide.value === "left" ? "right" : "left") as "left" | "right");
const selection = computed(() => picked.value[source.value]);
const selectionBytes = computed(() => pickedBytes.value[source.value]);

// Transfer state
const rows = ref<Row[]>([]);
const summary = ref<Summary | null>(null);
const runError = ref("");
const liveFile = ref<string | null>(null);
const stopping = ref(false);
const conflict = ref<ConflictAsk | null>(null);
const applyAll = ref(false);

const intent = ref<"move" | "copy">("move");
const syncing = ref(false);

const unlisten: Array<() => void> = [];

onMounted(async () => {
  places.value = await api.places();
  const home = places.value.find((p) => p.label === "Home")?.path ?? "/";
  const volume = places.value.find((p) => p.path.startsWith("/Volumes"))?.path;

  // Where you were last beats any guess we could make.
  const last = await api.lastPanes();
  leftStart.value = last.left ?? places.value.find((p) => p.label === "Movies")?.path ?? home;
  rightStart.value = last.right ?? volume ?? home;

  try {
    links.value = await api.links();
    const seen = await api.recent();
    onboarding.value = links.value.length === 0 && seen.length === 0;
  } catch (e) { error.value = String(e); }

  unlisten.push(
    await on.started((e) => {
      liveFile.value = e.path;
      rows.value.unshift({ path: e.path, size: e.size, state: "live", detail: null });
    }),
    await on.finished((e) => {
      const row = rows.value.find((r) => r.path === e.path && r.state === "live");
      if (row) { row.state = e.outcome; row.detail = e.detail; }
      if (liveFile.value === e.path) liveFile.value = null;
    }),
    await on.conflict((e) => { conflict.value = e; }),
    await on.done((e) => {
      summary.value = e; liveFile.value = null; stopping.value = false;
      left.value?.reload(); right.value?.reload();
      api.links().then((l) => (links.value = l)).catch(() => {});
    }),
    await on.failed((e) => { runError.value = e; stopping.value = false; }),
  );
});

onUnmounted(() => unlisten.forEach((f) => f()));

let saveTimer: number | undefined;
function located(side: "left" | "right", path: string) {
  paths.value = { ...paths.value, [side]: path };
  // Debounced: navigating quickly should not write the file on every step.
  clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    api.rememberPanes({ left: paths.value.left || null, right: paths.value.right || null })
      .catch(() => {});
  }, 400);
}

function record(side: "left" | "right", names: string[], total: number) {
  picked.value = { ...picked.value, [side]: names };
  pickedBytes.value = { ...pickedBytes.value, [side]: total };
}

function begin(which: "move" | "copy") {
  if (!selection.value.length) return;
  intent.value = which;
  syncing.value = true;
}

async function go(payload: Payload) {
  syncing.value = false;
  rows.value = [];
  summary.value = null;
  runError.value = "";
  tab.value = "transfers";
  try {
    await api.startTransfer({
      source: paths.value[source.value],
      destination: paths.value[target.value],
      names: selection.value,
      ...payload,
    });
  } catch (e) { runError.value = String(e); }
}

async function runSaved(name: string) {
  rows.value = []; summary.value = null; runError.value = "";
  tab.value = "transfers";
  try { await api.run(name); } catch (e) { runError.value = String(e); }
}

async function answer(action: string) {
  const all = applyAll.value;
  conflict.value = null; applyAll.value = false;
  try { await api.resolve(action, all); } catch (e) { runError.value = String(e); }
}

async function stop() {
  stopping.value = true;
  try { await api.cancel(); } catch (e) { runError.value = String(e); }
}

const running = computed(() => liveFile.value !== null);
</script>

<template>
  <div class="frame">
    <header class="topbar chrome">
      <div class="wordmark">tung<span>state</span></div>
      <nav class="tabs">
        <button :aria-current="tab === 'browse'" @click="tab = 'browse'">Browse</button>
        <button :aria-current="tab === 'transfers'" @click="tab = 'transfers'">
          Transfers<span v-if="running" class="badge">●</span>
        </button>
        <button :aria-current="tab === 'activity'" @click="tab = 'activity'">Activity</button>
      </nav>
      <span class="spacer"></span>
      <select v-if="links.length" class="btn small" style="max-width:190px"
              :value="''" @change="(e) => runSaved((e.target as HTMLSelectElement).value)">
        <option value="" disabled selected>Run a saved pair…</option>
        <option v-for="l in links" :key="l.name" :value="l.name">{{ l.name }}</option>
      </select>
    </header>

    <Welcome v-if="onboarding" @begin="onboarding = false" />

    <template v-else-if="tab === 'browse'">
      <div class="browser">
        <Pane ref="left" :start="leftStart" :places="places" :active="activeSide === 'left'"
              @focus="activeSide = 'left'"
              @selection="(n, t) => record('left', n, t)"
              @located="(p) => located('left', p)" />
        <Pane ref="right" :start="rightStart" :places="places" :active="activeSide === 'right'"
              @focus="activeSide = 'right'"
              @selection="(n, t) => record('right', n, t)"
              @located="(p) => located('right', p)" />
      </div>

      <footer class="actionbar chrome">
        <div class="tally">
          <template v-if="selection.length">
            <b>{{ selection.length }}</b> selected · {{ bytes(selectionBytes) }}
            <span style="color: var(--dust-dim)"> · from the {{ source }} side</span>
          </template>
          <template v-else>Select files, then move or copy them to the other side.</template>
        </div>
        <button class="btn" :disabled="!selection.length || running" @click="begin('copy')">
          Copy {{ source === "left" ? "→" : "←" }}
        </button>
        <button class="btn primary" :disabled="!selection.length || running" @click="begin('move')">
          Move {{ source === "left" ? "→" : "←" }}
        </button>
      </footer>
    </template>

    <TransfersView v-else-if="tab === 'transfers'" :rows="rows" :summary="summary"
                   :error="runError" :live="running" :stopping="stopping" @stop="stop" />

    <ActivityView v-else />

    <SyncModal v-if="syncing" :source="paths[source]" :destination="paths[target]"
               :names="selection" :total-bytes="selectionBytes" :intent="intent"
               @cancel="syncing = false" @start="go" />

    <div v-if="conflict" class="veil">
      <div class="modal" role="dialog" aria-modal="true">
        <div class="cap">
          <h2>That name is already taken</h2>
          <p class="why">A different file is already there, so one of these is not the one you want.</p>
        </div>
        <div class="body">
          <p class="subject">{{ conflict.path }}</p>
          <div class="compare">
            <div><span>Arriving</span><b>{{ bytes(conflict.incoming_size) }}</b></div>
            <div><span>Already there</span><b>{{ bytes(conflict.existing_size) }}</b></div>
          </div>
          <label class="check" style="margin-top:16px">
            <input type="checkbox" v-model="applyAll" />
            <span>Do this for every other clash in this transfer</span>
          </label>
        </div>
        <div class="feet">
          <button class="btn danger" @click="answer('replace')">Replace</button>
          <span class="spacer"></span>
          <button class="btn" @click="answer('skip')">Leave it here</button>
          <button class="btn" @click="answer('rename')">Keep both</button>
          <button class="btn primary" @click="answer('quarantine')">Set aside</button>
        </div>
      </div>
    </div>
  </div>
</template>
