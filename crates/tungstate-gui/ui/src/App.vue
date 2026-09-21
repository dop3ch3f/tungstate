<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, useTemplateRef } from "vue";
import Pane from "./components/Pane.vue";
import SyncModal, { type Payload } from "./components/SyncModal.vue";
import TransfersView, { type Row } from "./components/TransfersView.vue";
import LinksView from "./components/LinksView.vue";
import FindView from "./components/FindView.vue";
import StorageView from "./components/StorageView.vue";
import FoldersView from "./components/FoldersView.vue";
import ActivityView from "./components/ActivityView.vue";
import ConnectionsView from "./components/ConnectionsView.vue";
import Welcome from "./components/Welcome.vue";
import Mark from "./components/Mark.vue";
// --- slice 8, in progress ------------------------------------------------
// The new folder half, on 1; 0 returns to the app as it is today, so the two
// can be compared while the rest of the window is rebuilt around it. The Tauri
// window has no address bar, so a query string is not available. This block
// goes when the new frame replaces this file.
import FolderHalf from "./screens/folder/FolderHalf.vue";
import DialogHost from "./ui/DialogHost.vue";
const look = ref(true); // the new screen is the default while it is being refined; 0 shows the old app
function pickLook(e: KeyboardEvent) {
  if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
  if (e.key === "0") look.value = false;
  if (e.key === "1") look.value = true;
}
window.addEventListener("keydown", pickLook);
// --- end slice 8, temporary ----------------------------------------------

import { api, on, bytes, type Place, type Summary, type ConflictAsk, type Link, type Leg , type InterruptedRun, type Began, type IdenticalAsk, type Connection, type Accepted, type Preview} from "./api";

type Tab =
  | "browse"
  | "transfers"
  | "folders"
  | "links"
  | "connections"
  | "activity"
  | "find"
  | "storage";

const tab = ref<Tab>("browse");
const onboarding = ref(false);
const places = ref<Place[]>([]);
const links = ref<Link[]>([]);
const connections = ref<Connection[]>([]);
const error = ref("");

const left = useTemplateRef<InstanceType<typeof Pane>>("left");
const right = useTemplateRef<InstanceType<typeof Pane>>("right");

const leftStart = ref("");
const rightStart = ref("");
const activeSide = ref<"left" | "right">("left");

const picked = ref<{ left: string[]; right: string[] }>({ left: [], right: [] });
const pickedBytes = ref<{ left: number; right: number }>({ left: 0, right: 0 });
const paths = ref<{ left: string; right: string }>({ left: "", right: "" });

// The ticks decide the direction, not which pane happens to be focused. Both
// sides ticked means an exchange: each side's selection goes to the other.
const mode = computed<"none" | "right" | "left" | "exchange">(() => {
  const l = picked.value.left.length;
  const r = picked.value.right.length;
  if (l && r) return "exchange";
  if (l) return "right";
  if (r) return "left";
  return "none";
});

const legs = computed<Leg[]>(() => {
  const forward = { source: paths.value.left, destination: paths.value.right, names: picked.value.left };
  const back = { source: paths.value.right, destination: paths.value.left, names: picked.value.right };
  if (mode.value === "exchange") return [forward, back];
  if (mode.value === "right") return [forward];
  if (mode.value === "left") return [back];
  return [];
});

const count = computed(() => picked.value.left.length + picked.value.right.length);
const selectionBytes = computed(() => {
  if (mode.value === "exchange") return pickedBytes.value.left + pickedBytes.value.right;
  return mode.value === "right" ? pickedBytes.value.left : pickedBytes.value.right;
});
const arrow = computed(() =>
  mode.value === "exchange" ? "⇄" : mode.value === "left" ? "←" : "→",
);

// Transfer state
const rows = ref<Row[]>([]);
const summary = ref<Summary | null>(null);
const runError = ref("");
/// How many transfers are waiting behind the one running, when any are.
const queued = ref(0);
/// A saved link's preview, shown until dismissed. Nothing has happened yet.
const savedPreview = ref<{ name: string; preview: Preview } | null>(null);
/// The same words the sync dialog uses, for the same outcomes.
const previewWord: Record<string, string> = {
  move: "will move",
  check: "same size there",
  clash: "name taken",
  hold: "too recent",
};
const liveFile = ref<string | null>(null);
const stopping = ref(false);
const conflict = ref<ConflictAsk | null>(null);
const sameFile = ref<IdenticalAsk | null>(null);
const applyAll = ref(false);
const interrupted = ref<InterruptedRun[]>([]);
// From the engine, not from which button was pressed: a resumed run has no
// button behind it, and only the engine knows what the far side agreed to.
const shape = ref<Began | null>(null);
// Tracked apart from `shape` because the width changes during a run while
// everything else in `shape` is fixed at the start.
const atOnce = ref<number | null>(null);
const halting = ref(false);
const busy = ref("");

/// Work the last process left behind. Re-read whenever a run ends, so
/// finishing one clears its banner and a fresh failure grows one.
async function refreshInterrupted() {
  try { interrupted.value = await api.interrupted(); } catch { /* shown elsewhere */ }
}

async function resumeRun(link: string) {
  busy.value = link;
  rows.value = []; summary.value = null; runError.value = ""; discarded.value = ""; shape.value = null; atOnce.value = null;
  try { await api.resumeInterrupted(link); }
  catch (e) { runError.value = String(e); }
  finally { busy.value = ""; }
  await refreshInterrupted();
}

async function discardRun(link: string) {
  busy.value = link;
  try {
    const freed = await api.discardInterrupted(link);
    runError.value = "";
    discarded.value = `Cleaned up ${bytes(freed)} of part-copied files. The originals are untouched.`;
  } catch (e) { runError.value = String(e); }
  finally { busy.value = ""; }
  await refreshInterrupted();
}

const discarded = ref("");

const intent = ref<"move" | "copy">("move");
const syncing = ref(false);

const unlisten: Array<() => void> = [];

onMounted(async () => {
  // Listeners first, before anything that can throw. They were registered
  // last, after four awaits, and when `listen` itself was being refused the
  // whole of onMounted stopped there: browsing worked, transfers ran, and the
  // Transfers tab stayed empty for ever with nothing on screen to say why.
  try {
    await attach();
  } catch (e) {
    error.value =
      `This window cannot receive progress from the engine, so transfers will ` +
      `run without showing here: ${String(e)}`;
  }

  try {
    places.value = await api.places();
    connections.value = await api.connections();
    const home = places.value.find((p) => p.label === "Home")?.path ?? "/";
    // A connection first: it is the route that works whether or not the volume
    // is mounted, and it is the only one Windows and Linux ever get. Asked of
    // the connection list rather than sniffed out of a path, so nothing here
    // has to guess what a string means.
    const away = connections.value.length
      ? `${connections.value[0].name}:`
      : places.value.find((p) => p.path.startsWith("/Volumes"))?.path;

    // Where you were last beats any guess we could make.
    const last = await api.lastPanes();
    leftStart.value = last.left ?? places.value.find((p) => p.label === "Movies")?.path ?? home;
    rightStart.value = last.right ?? away ?? home;

    links.value = await api.links();
    const seen = await api.recent();
    onboarding.value = links.value.length === 0 && seen.length === 0;
    await refreshInterrupted();
    // Unfinished work is the first thing worth knowing about, ahead of
    // whichever folders the panes happen to be pointed at.
    if (interrupted.value.length) tab.value = "transfers";
  } catch (e) { error.value = String(e); }
});

async function attach() {
  unlisten.push(
    await on.began((e) => { shape.value = e; atOnce.value = e.at_once; }),
    await on.atOnce((n) => { atOnce.value = n; }),
    await on.checking((e) => {
      const row = rows.value.find((r) => r.path === e.path);
      if (row) { row.state = "checking"; row.done = e.done; row.checked = e.total; }
    }),
    // The whole plan arrives before the first file, so the list shows what is
    // waiting rather than growing one row at a time as things happen.
    await on.planned((files) => {
      rows.value = files.map((f) => ({
        path: f.path, size: f.size, state: "waiting", detail: null, done: 0, checked: 0,
      }));
    }),
    await on.started((e) => {
      liveFile.value = e.path;
      const row = rows.value.find((r) => r.path === e.path);
      if (row) { row.state = "live"; row.done = 0; row.checked = 0; }
      // A file the plan did not mention: possible when a conflict lands it
      // under another name. Better shown than dropped.
      else rows.value.push({ path: e.path, size: e.size, state: "live", detail: null, done: 0, checked: 0 });
    }),
    await on.advanced((e) => {
      const row = rows.value.find((r) => r.path === e.path);
      // Back to copying: a file that was being compared and is now sending
      // bytes has to lose the checking state or the row keeps the wrong word.
      if (row) { row.state = "live"; row.checked = 0; row.done = e.done; if (e.total) row.size = e.total; }
    }),
    await on.finished((e) => {
      const row = rows.value.find((r) => r.path === e.path && (r.state === "live" || r.state === "checking"));
      if (row) { row.state = e.outcome; row.detail = e.detail; row.done = row.size; }
      if (liveFile.value === e.path) liveFile.value = null;
    }),
    await on.conflict((e) => { conflict.value = e; }),
    await on.identical((e) => { sameFile.value = e; }),
    await on.done((e) => {
      summary.value = e; liveFile.value = null; stopping.value = false; halting.value = false;
      queued.value = 0;
      atOnce.value = null; sameFile.value = null;
      refreshInterrupted();
      left.value?.reload(); right.value?.reload();
      api.links().then((l) => (links.value = l)).catch(() => {});
    }),
    await on.queued((e) => began(e)),
    await on.failed((e) => { runError.value = e; stopping.value = false; halting.value = false; }),
  );
}

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
  if (mode.value === "none") return;
  intent.value = which;
  syncing.value = true;
}

async function go(payload: Payload) {
  syncing.value = false;
  tab.value = "transfers";
  try {
    await api.startTransfer({ legs: legs.value, ...payload });
  } catch (e) { runError.value = String(e); }
}

/// After a reset, restore or import, nothing on screen is still true.
async function reloadEverything() {
  try {
    links.value = await api.links();
    connections.value = await api.connections();
    await refreshInterrupted();
    left.value?.reload();
    right.value?.reload();
  } catch (e) { runError.value = String(e); }
}

async function refreshLinks() {
  try { links.value = await api.links(); } catch (e) { runError.value = String(e); }
}

/// Preview a saved link, in the same dialog a browser transfer uses.
async function previewSaved(name: string) {
  runError.value = "";
  try {
    savedPreview.value = { name, preview: await api.previewLink(name) };
  } catch (e) { runError.value = String(e); }
}

async function runSaved(name: string) {
  tab.value = "transfers";
  try { began(await api.run(name)); } catch (e) { runError.value = String(e); }
}

/// Clear the last run's progress, but only when a new run actually started.
/// Files added to a transfer already going share its rows, and blanking them
/// would erase the live view of the thing they just joined.
function began(accepted: Accepted) {
  if (accepted.started) {
    rows.value = []; summary.value = null; runError.value = "";
    shape.value = null; atOnce.value = null; queued.value = 0;
  } else {
    queued.value = accepted.waiting;
  }
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

async function stopNow() {
  halting.value = true;
  stopping.value = true;
  try { await api.stopNow(); } catch (e) { runError.value = String(e); }
}

async function answerIdentical(remove: boolean) {
  const all = applyAll.value;
  sameFile.value = null; applyAll.value = false;
  try { await api.resolveIdentical(remove, all); } catch (e) { runError.value = String(e); }
}

const running = computed(() => liveFile.value !== null);

/// Both lists move together: a connection appears in every pane's "Go to…"
/// the moment it exists, and disappears the moment it is forgotten.
async function refreshConnections() {
  try {
    connections.value = await api.connections();
    places.value = await api.places();
  } catch (e) { error.value = String(e); }
}

/// Sending a pane somewhere from the Connections tab has to switch tabs too,
/// or the navigation happens on a screen nobody is looking at.
function browseAt(location: string) {
  rightStart.value = location;
  activeSide.value = "right";
  tab.value = "browse";
}
</script>

<template>
  <template v-if="look"><FolderHalf /><DialogHost /></template>
  <div v-else class="frame">
    <header class="topbar chrome">
      <div class="wordmark"><Mark :size="20" /><span class="name">tung<span>state</span></span></div>
      <nav class="tabs">
        <button :aria-current="tab === 'browse'" @click="tab = 'browse'">Browse</button>
        <button :aria-current="tab === 'transfers'" @click="tab = 'transfers'">
          Transfers<span v-if="running" class="badge">●</span>
        </button>
        <button :aria-current="tab === 'folders'" @click="tab = 'folders'">Folders</button>
        <button :aria-current="tab === 'links'" @click="tab = 'links'">
          Links<span v-if="links.length" class="badge">{{ links.length }}</span>
        </button>
        <button :aria-current="tab === 'connections'" @click="tab = 'connections'">Connections</button>
        <button :aria-current="tab === 'activity'" @click="tab = 'activity'">Activity</button>
        <button :aria-current="tab === 'find'" @click="tab = 'find'">Find</button>
        <button :aria-current="tab === 'storage'" @click="tab = 'storage'">Storage</button>
      </nav>
      <span class="spacer"></span>
      <select v-if="links.length" class="btn small" style="max-width:190px"
              :value="''" @change="(e) => runSaved((e.target as HTMLSelectElement).value)">
        <option value="" disabled selected>Run a saved pair…</option>
        <option v-for="l in links" :key="l.name" :value="l.name">{{ l.name }}</option>
      </select>
    </header>

    <!-- Welcome is the empty state of Browse, not of the whole window. Gating
         the entire tab chain on it left every other tab dead on a fresh
         install: the tab bar still moved its `aria-current`, so the window
         said "you are on Folders" while showing onboarding. -->
    <Welcome v-if="onboarding && tab === 'browse'" @begin="onboarding = false" />

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
          <template v-if="mode === 'exchange'">
            <b>{{ picked.left.length }}</b> left and <b>{{ picked.right.length }}</b> right
            · {{ bytes(selectionBytes) }}
            <div class="route-line">each side goes to the other</div>
          </template>
          <template v-else-if="mode !== 'none'">
            <b>{{ count }}</b> selected · {{ bytes(selectionBytes) }}
            <div class="route-line">
              <b>{{ legs[0].source }}</b> {{ arrow }} <b>{{ legs[0].destination }}</b>
            </div>
          </template>
          <template v-else>Tick files on either side, then move or copy them across.</template>
        </div>
        <button class="btn" :disabled="mode === 'none' || running" @click="begin('copy')">
          {{ mode === "left" ? `${arrow} Copy` : `Copy ${arrow}` }}
        </button>
        <button class="btn primary" :disabled="mode === 'none' || running" @click="begin('move')">
          {{ mode === "left" ? `${arrow} Move` : `Move ${arrow}` }}
        </button>
      </footer>
    </template>

    <TransfersView v-else-if="tab === 'transfers'" :rows="rows" :summary="summary"
                   :error="runError" :live="running" :stopping="stopping"
                   :interrupted="interrupted" :busy="busy" :note="discarded" :shape="shape"
                   :at-once="atOnce" :halting="halting" :queued="queued"
                   @stop="stop" @stop-now="stopNow" @resume="resumeRun" @discard="discardRun" />

    <FindView v-else-if="tab === 'find'" :links="links" />

    <StorageView v-else-if="tab === 'storage'" @changed="reloadEverything" />

    <FoldersView v-else-if="tab === 'folders'" />

    <LinksView v-else-if="tab === 'links'" :links="links" :busy="running"
               :places="places.map((p) => p.path)"
               @changed="refreshLinks" @run="runSaved" @preview="previewSaved" />

    <ConnectionsView v-else-if="tab === 'connections'" :connections="connections"
                     @changed="refreshConnections" @browse="browseAt" />

    <!-- Last, and deliberately a bare `v-else`: it is the fallback, so a new
         tab value without its own branch above renders Activity in silence. -->
    <ActivityView v-else />

    <!-- A saved link's preview. Deliberately the same words the sync dialog
         uses for the same outcomes: the two answer the same question and
         should not read as different features. -->
    <div v-if="savedPreview" class="veil" @click.self="savedPreview = null">
      <div class="modal" role="dialog" aria-modal="true">
        <div class="cap">
          <h2>{{ savedPreview.name }}</h2>
          <p class="why">What running this would do. Nothing has happened yet.</p>
        </div>
        <div class="body">
          <div class="prospect">
            <div class="tallies">
              <div><b>{{ savedPreview.preview.fresh }}</b><span>will move</span></div>
              <div v-if="savedPreview.preview.same_size">
                <b>{{ savedPreview.preview.same_size }}</b><span>same size there</span>
              </div>
              <div v-if="savedPreview.preview.clashes">
                <b class="warn">{{ savedPreview.preview.clashes }}</b><span>name taken</span>
              </div>
              <div v-if="savedPreview.preview.too_recent">
                <b>{{ savedPreview.preview.too_recent }}</b><span>too recent</span>
              </div>
              <div><b>{{ bytes(savedPreview.preview.bytes) }}</b><span>to move</span></div>
            </div>
            <ul class="lines">
              <li v-for="(item, i) in savedPreview.preview.items.slice(0, 60)"
                  :key="item.path + i" :class="item.outcome">
                <span class="p">{{ item.path }}</span>
                <span class="w">{{ previewWord[item.outcome] }}</span>
                <span class="s">{{ bytes(item.size) }}</span>
              </li>
            </ul>
            <p v-if="savedPreview.preview.items.length > 60" class="note">
              and {{ savedPreview.preview.items.length - 60 }} more
            </p>
            <p class="note">
              {{ savedPreview.preview.removes_originals
                ? "Each original here is removed only after its copy passes the check."
                : "Nothing here is removed." }}
            </p>
          </div>
        </div>
        <div class="feet">
          <span class="spacer"></span>
          <button class="btn" @click="savedPreview = null">Close</button>
          <button class="btn primary" :disabled="running"
                  @click="runSaved(savedPreview.name); savedPreview = null">Run it</button>
        </div>
      </div>
    </div>

    <SyncModal v-if="syncing" :legs="legs" :count="count" :total-bytes="selectionBytes"
               :intent="intent" @cancel="syncing = false" @start="go" />

    <!-- Asked before the original here is removed, never after. A hash match
         is good evidence, but acting on it alone means this program deleting
         the user's other copy on the strength of its own arithmetic. -->
    <div v-if="sameFile" class="veil">
      <div class="modal" role="dialog" aria-modal="true">
        <div class="cap">
          <h2>This one is already there</h2>
          <p class="why">
            Both copies were read all the way through and they match exactly, so
            there is nothing to send. The only question left is the copy on this
            machine.
          </p>
        </div>
        <div class="body">
          <p class="subject">{{ sameFile.path }}</p>
          <div class="compare">
            <div><span>Both copies</span><b>{{ bytes(sameFile.size) }}</b></div>
          </div>
          <label class="check" style="margin-top:16px">
            <input type="checkbox" v-model="applyAll" />
            <span>Do this for every other file already there in this transfer</span>
          </label>
        </div>
        <div class="feet">
          <button class="btn" @click="answerIdentical(false)">Keep it here</button>
          <span class="spacer"></span>
          <button class="btn danger" @click="answerIdentical(true)">Remove it from here</button>
        </div>
      </div>
    </div>

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
