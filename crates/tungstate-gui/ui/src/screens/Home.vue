<!-- The window opens here, and this screen belongs to no one section.
     A status board rather than a menu: the headline says whether anything
     needs you, the alerts under it say what, and each section's tile says
     what it holds and when each thing last ran. Anything interrupted or
     broken sits above everything, because unfinished work is the one thing
     that should be seen before a choice is made. -->
<script setup lang="ts">
import { computed, onMounted, ref, shallowRef, watch } from "vue";
import { useNav } from "../nav";
import { useFolders } from "../state/useFolders";
import { useWatch } from "../state/useWatch";
import { useSync } from "../state/useSync";
import { useDupes } from "../state/useDupes";
import { useTransfer } from "../state/useTransfer";
import { folders, links, transfers, history, syncs, dupes } from "../engine/commands";
import { ago, bytes } from "../lib/format";
import { files, noticed } from "../lib/counts";
import { lately, leaf } from "../lib/lately";
import type { FolderView, InterruptedRun, Link, Op, PastRun, PastSync, Place, SyncView } from "../engine/types";
import Button from "../ui/Button.vue";
import Notice from "../ui/Notice.vue";
import TitleBar from "../ui/TitleBar.vue";

const nav = useNav();
const f = useFolders();
const w = useWatch();
const sy = useSync();
const dz = useDupes();
const t = useTransfer();

const governed = shallowRef<FolderView[]>([]);
const pairs = shallowRef<Link[]>([]);
const saved = shallowRef<SyncView[]>([]);
const scans = shallowRef<string[]>([]);
const places = shallowRef<Place[]>([]);
const stranded = shallowRef<InterruptedRun[]>([]);
const ops = shallowRef<Op[]>([]);
const tidies = shallowRef<PastRun[]>([]);
const cleanups = shallowRef<PastRun[]>([]);
const synced = shallowRef<PastSync[]>([]);
const problem = ref<string | null>(null);
const loaded = ref(false);

/** Rows per tile: enough to recognise, few enough that tiles stay short. */
const SHOWN = 3;
/** Lines under Lately: what just happened, not a second History. */
const LINES = 3;

// The watcher files things while this screen is open, and a Lately list
// saying nothing has happened under a tile saying it filed a file is one
// screen contradicting itself.
watch(w.recent, async () => {
  try {
    ops.value = await history.recent();
  } catch {
    // Lately keeps what it had; the watcher's own lines still say so.
  }
});

onMounted(async () => {
  try {
    const [fs, ls, ss, ds, ps, is, os, ts, cs, pss] = await Promise.all([
      folders.governed(),
      links.list(),
      syncs.list(),
      dupes.recent(),
      transfers.places(),
      transfers.interrupted(),
      history.recent(),
      folders.past(),
      dupes.past(),
      syncs.past(),
    ]);
    governed.value = fs;
    pairs.value = ls;
    saved.value = ss;
    scans.value = ds;
    places.value = ps;
    stranded.value = is;
    ops.value = os;
    tidies.value = ts;
    cleanups.value = cs;
    synced.value = pss;
  } catch (e) {
    problem.value = String(e);
  } finally {
    loaded.value = true;
  }
});

/** The newest of each key; the lists arrive newest first, so the first wins. */
function newest<T>(list: readonly T[], key: (item: T) => string | null): Map<string, T> {
  const seen = new Map<string, T>();
  for (const item of list) {
    const k = key(item);
    if (k !== null && !seen.has(k)) seen.set(k, item);
  }
  return seen;
}
const tidied = computed(() => newest(tidies.value, (r) => r.root));
const cleared = computed(() => newest(cleanups.value, (r) => r.root));
const ran = computed(() => newest(synced.value, (s) => s.sync));
const sent = computed(() => newest(ops.value, (op) => op.link));

const trouble = computed(() => w.recent.value.filter((n) => n.kind === "trouble"));
// A row with a fault says it in the alert's words, quietly: red is kept for
// the alert, so a fault is loud once.
const troubled = computed(() => new Map(trouble.value.map((n) => [n.folder, n.why])));
// Files waiting to be filed are news about a folder, not a fault: they are
// said on that folder's row rather than among what needs you.
const waiting = computed(() => new Map(w.recent.value.filter((n) => n.kind === "waiting").map((n) => [n.folder, n])));
const broken = computed(() => governed.value.filter((g) => g.broken));
const needs = computed(() => (stranded.value.length ? 1 : 0) + trouble.value.length + broken.value.length);
/** Nothing set up and nothing done: the first launch. */
const fresh = computed(() => loaded.value && !problem.value && !governed.value.length && !pairs.value.length
  && !saved.value.length && !scans.value.length && !ops.value.length);
const headline = computed(() => {
  if (!loaded.value) return "Home";
  if (problem.value) return "Something went wrong";
  if (fresh.value) return "Start here";
  const n = needs.value;
  return n === 0 ? "All in order" : `${n} ${n === 1 ? "thing needs" : "things need"} you`;
});

const roots = computed(() => new Map(governed.value.map((g) => [g.name, g.root])));
const lines = computed(() => lately(ops.value, w.recent.value, roots.value, LINES));

const downloads = computed(() => places.value.find((p) => p.label === "Downloads")?.path ?? null);

function openFolder(folder: FolderView) {
  nav.go("folder");
  void (folder.broken ? f.look(folder.root) : f.open(folder.root));
}
function openNamed(name: string) {
  const folder = governed.value.find((g) => g.name === name);
  if (folder) openFolder(folder);
  else nav.go("folder");
}
function look(path: string) {
  nav.go("folder");
  void f.look(path);
}
function chooseFolder() {
  nav.go("folder");
  void f.pick();
}
async function openSync(sync: SyncView) {
  nav.go("sync");
  await sy.load();
  sy.open(sync);
}
function newSync() {
  nav.go("sync");
  sy.making();
}
/** Pick up where it stopped, as the same words do on Transfer, and go and
 *  watch it there; a link that only led to a second button saying the same
 *  thing was one click too many. */
function pickUp(link: string) {
  void t.resume(link);
  t.openOn.value = "runs";
  nav.go("drain");
}
function openPairs() {
  t.openOn.value = "links";
  nav.go("drain");
}
function scanAgain(path: string) {
  nav.go("dupes");
  void dz.look(path);
}

</script>

<template>
  <div class="home">
    <div class="column">
      <h1>{{ headline }}</h1>

      <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
      <!-- One box for everything that needs you, each row with the one step
           that deals with it. A failure is red; a paused thing amber. -->
      <div class="needs" v-if="needs">
        <Notice tone="hold" v-if="stranded.length === 1">
          {{ stranded[0]!.link }}: stopped part-way.
          <template #act><Button look="link" @click="pickUp(stranded[0]!.link)">Resume</Button></template>
        </Notice>
        <Notice tone="hold" v-else-if="stranded.length">
          {{ stranded.length }} transfers stopped part-way.
          <template #act><Button look="link" @click="nav.go('drain')">See them</Button></template>
        </Notice>
        <Notice tone="bad" v-for="n in trouble" :key="`t${n.at}${n.folder}`">
          {{ n.folder }}: {{ n.why }}.
          <template #act><Button look="link" @click="openNamed(n.folder)">Open {{ n.folder }}</Button></template>
        </Notice>
        <Notice tone="bad" v-for="g in broken" :key="`b${g.root}`">
          <span :title="g.broken ?? undefined">{{ g.name }}: the rules file has a mistake.</span>
          <template #act><Button look="link" @click="openFolder(g)">Fix it</Button></template>
        </Notice>
      </div>

      <div class="grid">
        <section class="panel">
          <TitleBar class="pbar" of="folder" title="Organize">
            <Button look="link" v-if="governed.length" @click="chooseFolder()">Choose a folder…</Button>
            <Button look="link" v-if="governed.length > SHOWN" @click="nav.go('folder')">All {{ governed.length }}</Button>
          </TitleBar>
          <div class="pb">
            <ul class="rows" v-if="governed.length">
              <li v-for="folder in governed.slice(0, SHOWN)" :key="folder.root">
                <button class="row" :title="folder.root" @click="openFolder(folder)">
                  <span class="r-name">{{ folder.name }}</span>
                  <span class="r-fact" v-if="folder.broken">the rules file has a mistake</span>
                  <span class="r-fact" v-else-if="troubled.has(folder.name)">{{ troubled.get(folder.name) }}</span>
                  <span class="r-fact r-news" v-else-if="waiting.has(folder.name)">{{ files(noticed(waiting.get(folder.name)!)) }} new, waiting to be filed</span>
                  <span class="r-fact" v-else-if="folder.has_rules === false">no rules yet</span>
                  <span class="r-fact" v-else-if="tidied.has(folder.root)">tidied {{ ago(tidied.get(folder.root)!.applied_at) }}</span>
                  <span class="r-fact" v-else>not tidied yet</span>
                </button>
              </li>
            </ul>
            <template v-else-if="loaded">
              <p class="none">See how a folder is filed and tidy it, with every move undoable. Downloads is usually the messiest.</p>
              <footer class="pf">
                <Button v-if="downloads" look="primary" @click="look(downloads)">Start with Downloads</Button>
                <Button :look="downloads ? 'link' : undefined" @click="chooseFolder()">Choose a folder…</Button>
              </footer>
            </template>
          </div>
        </section>

        <section class="panel">
          <TitleBar class="pbar" of="drain" title="Transfer">
            <Button look="link" v-if="pairs.length" @click="nav.go('drain')">New transfer</Button>
            <Button look="link" v-if="pairs.length > SHOWN" @click="nav.go('drain')">All {{ pairs.length }}</Button>
          </TitleBar>
          <div class="pb">
            <ul class="rows" v-if="pairs.length">
              <li v-for="pair in pairs.slice(0, SHOWN)" :key="pair.name">
                <button class="row" :title="`${pair.source} → ${pair.destination}`" @click="openPairs()">
                  <span class="r-name">{{ pair.name }}</span>
                  <span class="r-fact" v-if="sent.has(pair.name)">sent {{ ago(sent.get(pair.name)!.started_at) }}</span>
                  <span class="r-fact" v-else>to {{ leaf(pair.destination) }}, not sent yet</span>
                </button>
              </li>
            </ul>
            <template v-else-if="loaded">
              <p class="none">Send files to a NAS or a drive. Each one is checked on arrival before any original is removed.</p>
              <footer class="pf"><Button @click="nav.go('drain')">New transfer</Button></footer>
            </template>
          </div>
        </section>

        <section class="panel">
          <TitleBar class="pbar" of="sync" title="Sync">
            <Button look="link" v-if="saved.length" @click="newSync()">New sync</Button>
            <Button look="link" v-if="saved.length > SHOWN" @click="nav.go('sync')">All {{ saved.length }}</Button>
          </TitleBar>
          <div class="pb">
            <ul class="rows" v-if="saved.length">
              <li v-for="sync in saved.slice(0, SHOWN)" :key="sync.name">
                <button class="row" :title="sync.members.map((m) => m.at).join(', ')" @click="openSync(sync)">
                  <span class="r-name">{{ sync.name }}</span>
                  <span class="r-fact" v-if="ran.has(sync.name)">ran {{ ago(ran.get(sync.name)!.applied_at) }}</span>
                  <span class="r-fact" v-else>not run yet</span>
                </button>
              </li>
            </ul>
            <template v-else-if="loaded">
              <p class="none">Keep a folder the same in two or more places.</p>
              <footer class="pf"><Button @click="newSync()">New sync</Button></footer>
            </template>
          </div>
        </section>

        <section class="panel">
          <TitleBar class="pbar" of="dupes" title="Duplicates">
            <Button look="link" v-if="scans.length" @click="nav.go('dupes')">Find duplicates</Button>
          </TitleBar>
          <div class="pb">
            <ul class="rows" v-if="scans.length">
              <li v-for="path in scans.slice(0, SHOWN)" :key="path">
                <button class="row" :title="`Look through ${path} again`" @click="scanAgain(path)">
                  <span class="r-name">{{ leaf(path) }}</span>
                  <span class="r-fact" v-if="cleared.has(path)">cleared {{ bytes(cleared.get(path)!.bytes) }} {{ ago(cleared.get(path)!.applied_at) }}</span>
                  <span class="r-fact" v-else>nothing cleared</span>
                </button>
              </li>
            </ul>
            <template v-else-if="loaded">
              <p class="none">Find files you have more than once, and clear the extras.</p>
              <footer class="pf"><Button @click="nav.go('dupes')">Find duplicates</Button></footer>
            </template>
          </div>
        </section>
      </div>

      <section class="panel" v-if="lines.length">
        <TitleBar class="pbar" of="history" title="Lately">
          <Button look="link" @click="nav.go('history')">See all</Button>
        </TitleBar>
        <div class="pb">
          <ul class="rows">
            <li v-for="line in lines" :key="`${line.at}${line.text}`" class="op">
              <span class="o-text">{{ line.text }}<span class="o-by" v-if="line.by"> automatically</span></span>
              <span class="o-when">{{ ago(line.at) }}</span>
            </li>
          </ul>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.home { position: absolute; inset: 0; overflow-y: auto; scrollbar-gutter: stable; }
.column { max-width: 900px; margin: 0 auto; padding: var(--s5) var(--s6) var(--s6); display: flex; flex-direction: column; gap: var(--s4); }
h1 { font-size: var(--title); font-weight: 700; letter-spacing: -0.01em; margin: 0; }
/* Each thing that needs you is a notice of its own, with the one step that
   deals with it. */
.needs { display: flex; flex-direction: column; gap: var(--s2); }

/* Each tile as tall as what it holds, so a busy Home still fits one screen. */
.grid { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: var(--s4); align-items: start; }
.panel {
  display: flex;
  flex-direction: column;
  background: var(--panel);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius-lg);
  min-width: 0;
  overflow: hidden;
  box-shadow: var(--lift-panel);
}
.panel > .pbar { padding: var(--s4) var(--s4) 0; }
.pb { display: flex; flex-direction: column; padding: var(--s2) var(--s4) var(--s4); }
/* Retro: the bar is flush with the window's edges, and the body gets room. */
:global([data-theme="retro"] .panel > .pbar) { padding: 6px 7px 6px var(--s3); }
:global([data-theme="retro"] .pb) { padding-top: var(--s2); padding-bottom: var(--s2); }
/* The last row needs no rule of its own: the tile's edge is one. */
.rows { list-style: none; margin: 0; padding: 0; }
.rows li + li { border-top: 1px solid var(--rule); }
.row {
  display: flex;
  align-items: baseline;
  gap: var(--s3);
  width: 100%;
  font: inherit;
  text-align: left;
  color: inherit;
  background: none;
  border: none;
  padding: 8px var(--s1);
  cursor: pointer;
  min-width: 0;
}
.row:hover { background: var(--surface-hover); }
.r-name { font-size: var(--small); font-weight: 600; flex: none; }
.r-fact { margin-left: auto; font-size: var(--fine); color: var(--text-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
.r-news { color: var(--hold); }
.none { font-size: var(--small); color: var(--text-quiet); margin: var(--s2) 0 0; line-height: 1.5; }
.pf { display: flex; align-items: center; gap: var(--s3); margin: var(--s4) 0 var(--s2); }

.op { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: var(--s3); align-items: center; padding: 8px var(--s1); font-size: var(--small); }
.o-text { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.o-by { color: var(--text-faint); }
.o-when { color: var(--text-faint); font-size: var(--fine); }
</style>
