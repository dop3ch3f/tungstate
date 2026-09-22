<!-- The window opens here, and this screen belongs to neither half.
     An overview rather than a menu: the folders you have, the pairs you have
     saved, and what happened last, each one click from acting on it. Anything
     interrupted sits above everything, because unfinished work is the one
     thing that should be seen before a choice is made. -->
<script setup lang="ts">
import { onMounted, ref, shallowRef } from "vue";
import { useNav } from "../nav";
import { useFolders } from "../state/useFolders";
import { folders, links, transfers, history } from "../engine/commands";
import { shortPath, when } from "../lib/format";
import { toneOfStatus } from "../lib/tone";
import type { FolderView, InterruptedRun, Link, Op } from "../engine/types";
import Button from "../ui/Button.vue";
import Notice from "../ui/Notice.vue";
import Tile from "../ui/Tile.vue";

const nav = useNav();
const f = useFolders();
const governed = shallowRef<FolderView[]>([]);
const pairs = shallowRef<Link[]>([]);
const stranded = shallowRef<InterruptedRun[]>([]);
const recent = shallowRef<Op[]>([]);
const problem = ref<string | null>(null);

onMounted(async () => {
  try {
    const [fs, ls, is, rs] = await Promise.all([
      folders.governed(),
      links.list(),
      transfers.interrupted(),
      history.recent(),
    ]);
    governed.value = fs;
    pairs.value = ls;
    stranded.value = is;
    recent.value = rs.slice(0, 6);
  } catch (e) {
    problem.value = String(e);
  }
});

function openFolder(folder: FolderView) {
  nav.go("folder");
  void (folder.broken ? f.look(folder.root) : f.open(folder.root));
}
function pointAtFolder() {
  nav.go("folder");
  void f.pick();
}

const leaf = (path: string | null) => (path ?? "").split("/").pop() ?? "";
// A lookup, because the CSS checker cannot follow a computed class name.
const DOT = { plain: "o-plain", live: "o-live", ok: "o-ok", hold: "o-hold", bad: "o-bad" } as const;
</script>

<template>
  <div class="home">
    <div class="column">
      <h1>Home</h1>

      <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
      <Notice tone="hold" v-if="stranded.length">
        {{ stranded.length === 1 ? "A transfer" : `${stranded.length} transfers` }}
        stopped part-way and can be picked up.
        <Button look="link" @click="nav.go('drain')">Look at it</Button>
      </Notice>

      <div class="pair">
        <section class="panel">
          <header class="ph">
            <Tile of="folder" :size="28" />
            <div class="ph-words">
              <h2>Tidy a folder</h2>
              <p>See how a folder is filed, beside every other way to file it.</p>
            </div>
          </header>
          <ul class="rows" v-if="governed.length">
            <li v-for="folder in governed.slice(0, 4)" :key="folder.root">
              <button class="row" @click="openFolder(folder)">
                <span class="r-name">{{ folder.name }}</span>
                <span class="path r-path">{{ shortPath(folder.root) }}</span>
                <span class="r-bad" v-if="folder.broken">rules will not load</span>
                <span class="r-dim" v-else-if="!folder.has_rules">no rules yet</span>
              </button>
            </li>
          </ul>
          <p class="none" v-else>No folders yet. Downloads is usually the messiest one.</p>
          <footer class="pf">
            <Button @click="pointAtFolder()">Point at a folder…</Button>
            <Button look="link" v-if="governed.length > 4" @click="nav.go('folder')">All {{ governed.length }}</Button>
          </footer>
        </section>

        <section class="panel">
          <header class="ph">
            <Tile of="drain" :size="28" />
            <div class="ph-words">
              <h2>Move to another machine</h2>
              <p>Copy, check it arrived intact, then remove the original.</p>
            </div>
          </header>
          <ul class="rows" v-if="pairs.length">
            <li v-for="pair in pairs.slice(0, 4)" :key="pair.name">
              <button class="row" @click="nav.go('drain')">
                <span class="r-name">{{ pair.name }}</span>
                <span class="path r-path">{{ shortPath(pair.source) }} → {{ shortPath(pair.destination) }}</span>
              </button>
            </li>
          </ul>
          <p class="none" v-else>No saved pairs yet. Pick files in the browser and name the pair when you send them.</p>
          <footer class="pf">
            <Button @click="nav.go('drain')">Open the file browser</Button>
          </footer>
        </section>
      </div>

      <section class="panel">
        <header class="ph">
          <Tile of="history" :size="28" />
          <div class="ph-words">
            <h2>What has happened</h2>
          </div>
          <Button look="link" class="ph-more" @click="nav.go('history')">Everything</Button>
        </header>
        <ul class="rows" v-if="recent.length">
          <li v-for="op in recent" :key="op.id" class="op">
            <span class="o-dot" :class="DOT[toneOfStatus(op.status)]"></span>
            <span class="o-kind">{{ op.kind }}</span>
            <span class="o-leaf">{{ leaf(op.destination ?? op.source) }}</span>
            <span class="o-when">{{ when(op.started_at) }}</span>
          </li>
        </ul>
        <p class="none" v-else>Nothing has happened yet.</p>
      </section>
    </div>
  </div>
</template>

<style scoped>
.home { position: absolute; inset: 0; overflow-y: auto; scrollbar-gutter: stable; }
.column { max-width: 900px; margin: 0 auto; padding: var(--s5) var(--s6) var(--s6); display: flex; flex-direction: column; gap: var(--s4); }
h1 { font-size: var(--title); font-weight: 700; letter-spacing: -0.01em; margin: 0; }

.pair { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: var(--s4); }
.panel {
  display: flex;
  flex-direction: column;
  background: var(--panel);
  border: 1px solid var(--edge);
  border-radius: var(--radius-lg);
  padding: var(--s4);
  min-width: 0;
  box-shadow: var(--lift-panel);
}
/* Retro: each panel is a little window, its header a title bar. The whole
   selector sits inside :global(), because Vue drops anything after it. */
:global([data-theme="retro"] .ph) {
  margin: calc(var(--s4) * -1) calc(var(--s4) * -1) 0;
  padding: var(--s2) var(--s4);
  background: var(--surface-raised);
  border-bottom: 1px solid var(--edge);
  border-radius: var(--radius-lg) var(--radius-lg) 0 0;
  align-items: center;
}
:global([data-theme="retro"] .ph p) { display: none; }
/* Three window controls in the retro primaries, drawn as one dot and two
   shadows of it, ahead of the tile. */
:global([data-theme="retro"] .ph::before) {
  content: "";
  flex: none;
  width: 9px;
  height: 9px;
  margin-right: 30px;
  border-radius: 50%;
  background: var(--tint-folder);
  box-shadow: 14px 0 0 var(--tint-home), 28px 0 0 var(--tint-drain);
}
:global([data-theme="retro"] .rows) { border-top: none; }
.ph { display: flex; gap: var(--s3); align-items: flex-start; }
.ph-words { min-width: 0; flex: 1; }
.ph h2 { font-size: var(--body); font-weight: 600; margin: 3px 0 0; }
.ph p { font-size: var(--small); color: var(--text-quiet); margin: 2px 0 0; line-height: 1.45; }
.ph-more { align-self: center; font-size: var(--small); }

.rows { list-style: none; margin: var(--s3) 0 0; padding: 0; border-top: 1px solid var(--edge); flex: 1; }
.rows li { border-bottom: 1px solid var(--edge); }
.row {
  display: flex;
  align-items: baseline;
  gap: var(--s2);
  width: 100%;
  font: inherit;
  text-align: left;
  color: inherit;
  background: none;
  border: none;
  padding: 7px var(--s1);
  cursor: pointer;
  min-width: 0;
}
.row:hover { background: var(--surface-hover); }
.r-name { font-size: var(--small); font-weight: 600; flex: none; }
.r-path { color: var(--text-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; word-break: normal; min-width: 0; }
.r-bad { margin-left: auto; flex: none; font-size: var(--fine); color: var(--bad); }
.r-dim { margin-left: auto; flex: none; font-size: var(--fine); color: var(--text-faint); }
.none { font-size: var(--small); color: var(--text-faint); margin: var(--s3) 0 0; flex: 1; line-height: 1.5; }
.pf { display: flex; align-items: center; gap: var(--s3); margin-top: var(--s4); }

.op { display: grid; grid-template-columns: 8px 80px minmax(0, 1fr) auto; gap: var(--s3); align-items: center; padding: 7px var(--s1); font-size: var(--small); }
.o-dot { width: 7px; height: 7px; border-radius: 50%; }
.o-plain { background: var(--text-faint); }
.o-live { background: var(--accent); }
.o-ok { background: var(--ok); }
.o-hold { background: var(--hold); }
.o-bad { background: var(--bad); }
.o-kind { color: var(--text-faint); }
.o-leaf { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.o-when { color: var(--text-faint); font-size: var(--fine); }
</style>
