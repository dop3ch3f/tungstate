<!-- Settings. For now, what Tungstate remembers: export it, import it, start
     fresh, and bring back anything archived. None of this touches your files;
     it only changes what the app remembers about them. Also how the window
     looks. -->
<script setup lang="ts">
import { onMounted, ref, shallowRef } from "vue";
import { storage } from "../engine/commands";
import { useWatch } from "../state/useWatch";
import { useTheme } from "../state/useTheme";
import { THEMES } from "../lib/theme";
import { useUpdate } from "../state/useUpdate";
import { getVersion } from "@tauri-apps/api/app";
import type { ArchiveView } from "../engine/types";
import { bytes } from "../lib/format";
import { ask } from "../ui/useDialog";
import Button from "../ui/Button.vue";
import Notice from "../ui/Notice.vue";
import Tile from "../ui/Tile.vue";
import Toggle from "../ui/Toggle.vue";
import RowMenu from "../ui/RowMenu.vue";

const archives = shallowRef<ArchiveView[]>([]);
const problem = ref<string | null>(null);
const said = ref<string | null>(null);
const busy = ref(false);
const w = useWatch();
const look = useTheme();
const up = useUpdate();
const version = ref<string | null>(null);

async function load() {
  try {
    archives.value = await storage.archives();
  } catch (e) {
    problem.value = String(e);
  }
}
onMounted(() => {
  void getVersion().then((v) => (version.value = v)).catch(() => {});
  void load();
  void w.load();
});

/** Every action here reports one sentence or one error, and never both. */
async function act(work: () => Promise<string | null>) {
  busy.value = true;
  problem.value = null;
  said.value = null;
  try {
    said.value = await work();
    await load();
  } catch (e) {
    problem.value = String(e);
  } finally {
    busy.value = false;
  }
}

const exportTo = () =>
  act(async () => {
    const path = await storage.pickSaveFile("tungstate-export.json");
    if (!path) return null;
    await storage.export(path);
    return `Written to ${path}. It carries no passwords; those stay in your keychain.`;
  });

const importFrom = () =>
  act(async () => {
    const path = await storage.pickOpenFile();
    if (!path) return null;
    const rows = await storage.import(path);
    return `Read ${rows} rows from ${path}. Re-enter any connection passwords under Transfer, Connections.`;
  });

async function startFresh() {
  const answer = await ask({
    title: "Start fresh?",
    why: "Connections, saved pairs and history are archived, then cleared. No file changes, and the archive can bring it all back.",
    choices: [
      { id: "no", label: "Keep everything" },
      { id: "yes", label: "Archive and start fresh", look: "danger" },
    ],
  });
  if (answer.id !== "yes") return;
  await act(async () => {
    const name = await storage.reset();
    void name;
    return "Everything was archived first, so it can be brought back. Nothing was deleted.";
  });
}

const restore = (archive: ArchiveView) =>
  act(async () => {
    const aside = await storage.restore(archive.name);
    void aside;
    return `Restored the archive from ${stamp(archive.archived_at)}. What was here was archived first, so this can be undone too.`;
  });

async function forget(archive: ArchiveView) {
  const answer = await ask({
    title: `Delete the archive from ${stamp(archive.archived_at)}?`,
    why: "This archive is gone for good. Your files are not affected.",
    choices: [
      { id: "no", label: "Keep it" },
      { id: "yes", label: "Delete", look: "danger" },
    ],
  });
  if (answer.id !== "yes") return;
  await act(async () => {
    await storage.forget(archive.name);
    return `Deleted the archive from ${stamp(archive.archived_at)}.`;
  });
}

/** One human date for an archive, the same one its questions use. */
const stamp = (ms: number) =>
  ms ? new Date(ms).toLocaleString(undefined, { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" }) : "an unknown time";
const clock = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
// A lookup, because the CSS checker cannot follow a computed class name.
const SWATCH = {
  system: "sw-system",
  retro: "sw-retro",
  "retro-dark": "sw-retro-dark",
  graphite: "sw-graphite",
  paper: "sw-paper",
} as const;
</script>

<template>
  <div class="st-wrap">
    <div class="st-column">
      <div class="head"><Tile of="settings" :size="26" /><h1>Settings</h1></div>

      <section class="st-sec">
        <h2>Appearance</h2>
        <div class="st-themes" role="radiogroup" aria-label="Appearance">
          <button
            v-for="t in THEMES"
            :key="t.id"
            class="st-theme"
            :class="{ 'st-theme-on': look.choice.value === t.id }"
            role="radio"
            :aria-checked="look.choice.value === t.id"
            @click="look.choose(t.id)"
          >
            <span class="st-swatch" :class="SWATCH[t.id]" aria-hidden="true"><i></i><i></i><i v-if="t.id === 'system'"></i><i v-if="t.id === 'system'"></i></span>
            <span class="st-tick" v-if="look.choice.value === t.id" aria-hidden="true">✓</span>
            <b>{{ t.label }}</b>
            <em>{{ t.note }}</em>
          </button>
        </div>
      </section>

      <section class="st-sec">
        <h2>While Tungstate is open</h2>
        <div class="st-row">
          <div class="st-say">
            <b>Keep folders in order</b>
            <em>Folders set to tidy themselves are sorted as files arrive; the others are only noted. It stops when you close the window.</em>
          </div>
          <Toggle :on="w.on.value" label="Keep folders in order" @change="(on) => w.set(on)" />
        </div>
        <p class="st-why" v-if="w.on.value && w.running.value">
          Watching {{ w.watching.value }} folder{{ w.watching.value === 1 ? "" : "s" }}<template
            v-if="w.sweeping.value"
          >, and checking {{ w.sweeping.value }} on a share every hour instead, because a
          share says nothing about a change another machine made</template>.
        </p>
        <p class="st-why" v-else-if="w.on.value">Nothing to watch yet. Add a folder under Organize.</p>
      </section>

      <section class="st-sec">
        <h2>This version</h2>
        <div class="st-row">
          <div class="st-say">
            <b>Tungstate {{ version ?? "" }}</b>
            <em v-if="up.ready.value">{{ up.ready.value.version }} is ready to install.</em>
            <em v-else-if="up.checkedAt.value">Up to date, checked at {{ clock(up.checkedAt.value) }}. Looked for when the app opens, and once a day.</em>
            <em v-else>Looked for when the app opens, and once a day.</em>
          </div>
          <Button look="primary" v-if="up.ready.value" @click="up.showing.value = true">Update and restart…</Button>
          <Button v-else :busy="up.checking.value" @click="up.look()">Look for updates</Button>
        </div>
        <Notice tone="bad" v-if="up.problem.value && !up.showing.value">{{ up.problem.value }}</Notice>
      </section>

      <!-- Everything about what the app keeps, the one thing that clears it last. -->
      <section class="st-sec">
        <h2>What Tungstate remembers</h2>
        <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
        <Notice v-if="said">{{ said }}</Notice>
        <div class="st-row">
          <div class="st-say">
            <b>Connections, saved pairs and history</b>
            <em>The record of every file it has moved. Never your files themselves.</em>
          </div>
          <span class="st-do">
            <Button :disabled="busy" @click="importFrom()">Import…</Button>
            <Button :busy="busy" @click="exportTo()">Export…</Button>
          </span>
        </div>

        <div class="st-say st-sub">
          <b>Archives</b>
          <em>Starting fresh or restoring saves what was here first. An archive goes only if you delete it.</em>
        </div>
        <p class="st-none" v-if="!archives.length">Nothing archived yet.</p>
        <ul class="st-list" v-else>
          <li class="st-arch" v-for="a in archives" :key="a.name">
            <span class="st-when">{{ stamp(a.archived_at) }}</span>
            <span class="st-holds" v-if="a.operations !== null">
              {{ a.links }} saved pair{{ a.links === 1 ? "" : "s" }}, {{ a.connections }} connection{{ a.connections === 1 ? "" : "s" }},
              {{ a.operations }} file {{ a.operations === 1 ? "move" : "moves" }} recorded
            </span>
            <span class="st-holds st-bad" v-else>cannot be read</span>
            <span class="num st-size">{{ bytes(a.size) }}</span>
            <span class="st-act">
              <Button :disabled="busy || a.operations === null" @click="restore(a)">Restore</Button>
              <RowMenu :label="`More for the archive from ${stamp(a.archived_at)}`" :items="[{ id: 'delete', label: 'Delete', danger: true }]" @pick="forget(a)" />
            </span>
          </li>
        </ul>

        <div class="st-row st-sub">
          <div class="st-say">
            <b>Start fresh</b>
            <em>Clears what Tungstate remembers, archiving it first. No file on this Mac or the NAS changes.</em>
          </div>
          <Button look="danger" :disabled="busy" @click="startFresh()">Start fresh…</Button>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.st-wrap { position: absolute; inset: 0; overflow-y: auto; scrollbar-gutter: stable; }
/* One measure for every section, lists included. */
.st-column { padding: var(--win-pad); max-width: 820px; }
.head { display: flex; align-items: center; gap: var(--s3); }
h1 { font-size: var(--display); font-weight: 700; margin: 0; letter-spacing: -0.01em; }
.st-sec { margin-top: var(--s6); display: flex; flex-direction: column; gap: var(--s3); }
.st-sec h2 { font-size: var(--body); font-weight: 700; margin: 0; padding-bottom: var(--s2); border-bottom: 1px solid var(--rule); }
.st-why { font-size: var(--small); color: var(--text-quiet); margin: 0; line-height: 1.5; }
.st-do { display: flex; gap: var(--s2); flex: none; }
.st-sub { margin-top: var(--s3); }
.st-none { font-size: var(--small); color: var(--text-faint); margin: 0; }
/* The chosen one's mark, in the tile's corner over its swatch. */
.st-tick {
  position: absolute;
  top: calc(var(--s2) + 7px);
  right: calc(var(--s2) + 5px);
  display: grid;
  place-items: center;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  font-size: var(--fine);
  font-weight: 700;
  color: var(--panel);
  background: var(--text);
  border: var(--bw) solid var(--panel);
}

/* A setting: what it is on the left, its control on the right. */
.st-row { display: flex; align-items: center; justify-content: space-between; gap: var(--s5); }
.st-say { display: flex; flex-direction: column; gap: 2px; min-width: 0; font-size: var(--small); }
.st-say em { font-style: normal; font-size: var(--fine); color: var(--text-faint); line-height: 1.5; }

/* Each theme as a small card with its own colours in miniature. */
.st-themes { display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: var(--s3); }
.st-theme {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  padding: var(--s2);
  font: inherit;
  font-size: var(--small);
  text-align: left;
  color: var(--text);
  background: none;
  border: var(--bw) solid var(--rule);
  border-radius: var(--radius);
  cursor: pointer;
}
.st-theme em { font-style: normal; font-size: var(--fine); color: var(--text-faint); line-height: 1.4; }
.st-theme:hover { border-color: var(--edge); }
/* Chosen as the rail marks the section you are in: filled, outlined, raised. */
.st-theme-on { background: var(--chosen); color: var(--chosen-ink); border-color: var(--edge); box-shadow: var(--lift); }
.st-theme-on em { color: var(--text-quiet); }
.st-swatch { display: flex; width: 100%; height: 34px; margin-bottom: var(--s1); border: 1px solid var(--edge); border-radius: var(--radius); overflow: hidden; }
.st-swatch i { flex: 1; }
.sw-retro i:first-child { background: var(--swatch-retro-desk); }
.sw-retro i:last-child { background: var(--swatch-retro-ink); flex: 0 0 30%; }
.sw-retro-dark i:first-child { background: var(--swatch-retro-dark-desk); }
.sw-retro-dark i:last-child { background: var(--swatch-retro-dark-ink); flex: 0 0 30%; }
.sw-graphite i:first-child { background: var(--swatch-graphite-desk); }
.sw-graphite i:last-child { background: var(--swatch-graphite-ink); flex: 0 0 30%; }
.sw-paper i:first-child { background: var(--swatch-paper-desk); }
.sw-paper i:last-child { background: var(--swatch-paper-ink); flex: 0 0 30%; }
/* Matching the system is both retros side by side: light, then dark. */
.sw-system i:nth-child(1) { background: var(--swatch-retro-desk); }
.sw-system i:nth-child(2) { background: var(--swatch-retro-ink); flex: 0 0 15%; }
.sw-system i:nth-child(3) { background: var(--swatch-retro-dark-desk); }
.sw-system i:nth-child(4) { background: var(--swatch-retro-dark-ink); flex: 0 0 15%; }


.st-list { list-style: none; margin: 0; padding: 0; }
.st-arch {
  display: grid;
  grid-template-columns: 170px minmax(0, 1fr) 70px auto;
  gap: var(--s3);
  align-items: center;
  padding: var(--s2) 0;
  border-bottom: 1px solid var(--rule);
  font-size: var(--small);
}
.st-when { font-weight: 600; }
.st-holds { font-size: var(--fine); color: var(--text-quiet); }
.st-bad { color: var(--bad); }
.st-size { font-size: var(--fine); color: var(--text-faint); text-align: right; }
.st-act { display: flex; align-items: center; gap: var(--s3); }
</style>
