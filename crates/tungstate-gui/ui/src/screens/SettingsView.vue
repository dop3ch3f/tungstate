<!-- Settings. For now, what Tungstate remembers: export it, import it, start
     fresh, and bring back anything archived. None of this touches your files;
     it only changes what the app remembers about them. A theme switch joins
     this screen later (docs/SYLLABUS.md, 15+). -->
<script setup lang="ts">
import { onMounted, ref, shallowRef } from "vue";
import { storage } from "../engine/commands";
import { useWatch } from "../state/useWatch";
import type { ArchiveView } from "../engine/types";
import { bytes } from "../lib/format";
import { ask } from "../ui/useDialog";
import Button from "../ui/Button.vue";
import Notice from "../ui/Notice.vue";
import Empty from "../ui/Empty.vue";
import Tile from "../ui/Tile.vue";

const archives = shallowRef<ArchiveView[]>([]);
const problem = ref<string | null>(null);
const said = ref<string | null>(null);
const busy = ref(false);
const w = useWatch();

async function load() {
  try {
    archives.value = await storage.archives();
  } catch (e) {
    problem.value = String(e);
  }
}
onMounted(() => {
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
    why: "Your connections, saved pairs and history are archived first, then cleared. No file on disk or on the NAS changes, and you can bring it all back from the list below.",
    choices: [
      { id: "no", label: "Keep everything" },
      { id: "yes", label: "Archive and start fresh", look: "danger" },
    ],
  });
  if (answer.id !== "yes") return;
  await act(async () => {
    const name = await storage.reset();
    return `Everything was archived as ${name}. Nothing was deleted.`;
  });
}

const restore = (archive: ArchiveView) =>
  act(async () => {
    const aside = await storage.restore(archive.name);
    return `Restored ${archive.name}. What was here is archived as ${aside}, so this can be undone too.`;
  });

async function forget(archive: ArchiveView) {
  const answer = await ask({
    title: `Delete ${archive.name}?`,
    why: "This archive is gone for good. Your files are not affected.",
    choices: [
      { id: "no", label: "Keep it" },
      { id: "yes", label: "Delete", look: "danger" },
    ],
  });
  if (answer.id !== "yes") return;
  await act(async () => {
    await storage.forget(archive.name);
    return `Deleted ${archive.name}.`;
  });
}

const stamp = (ms: number) => (ms ? new Date(ms).toLocaleString() : "unknown");
</script>

<template>
  <div class="st-wrap">
    <div class="st-column">
      <div class="head"><Tile of="settings" :size="26" /><h1>Settings</h1></div>

      <section class="st-sec">
        <h2>Keeping folders in order</h2>
        <label class="st-switch">
          <input
            type="checkbox"
            :checked="w.on.value"
            @change="w.set(($event.target as HTMLInputElement).checked)"
          />
          <span>
            Keep folders in order while Tungstate is open
            <em>
              Folders you have set to enforce are tidied on their own as files
              arrive and settle. The rest are only ever told about. There is no
              background service yet, so this stops when you close the window.
            </em>
          </span>
        </label>
        <p class="st-why" v-if="w.on.value && w.running.value">
          Watching {{ w.watching.value }} folder{{ w.watching.value === 1 ? "" : "s" }}<template
            v-if="w.sweeping.value"
          >, and checking {{ w.sweeping.value }} on a share every hour instead, because a
          share says nothing about a change another machine made</template>.
        </p>
        <p class="st-why" v-else-if="w.on.value">
          Nothing to watch yet. Add a folder under Organize.
        </p>
      </section>

      <section class="st-sec">
        <h2>What Tungstate remembers</h2>
        <p class="st-why">
          Your connections, saved pairs, and the record of every file it has moved.
          None of this is your files: clearing it changes nothing on disk.
        </p>
        <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
        <Notice v-if="said">{{ said }}</Notice>
        <div class="st-do">
          <Button :disabled="busy" @click="exportTo()">Export…</Button>
          <Button :disabled="busy" @click="importFrom()">Import…</Button>
          <Button look="danger" :disabled="busy" @click="startFresh()">Start fresh…</Button>
        </div>
      </section>

      <section class="st-sec">
        <h2>Archives</h2>
        <p class="st-why">Starting fresh or restoring writes out what was here first. Nothing is thrown away unless you delete it here.</p>
        <Empty v-if="!archives.length" art="no-archives" line="Nothing archived yet." />
        <div class="st-row" v-for="a in archives" :key="a.name">
          <div class="st-who">
            <span class="st-when">{{ stamp(a.archived_at) }}</span>
            <span class="st-name">{{ a.name }}</span>
          </div>
          <span class="st-holds" v-if="a.operations !== null">
            {{ a.links }} saved pair{{ a.links === 1 ? "" : "s" }}, {{ a.connections }} connection{{ a.connections === 1 ? "" : "s" }}, {{ a.operations }} recorded
          </span>
          <span class="st-holds st-bad" v-else>cannot be read</span>
          <span class="num st-size">{{ bytes(a.size) }}</span>
          <div class="st-act">
            <Button :disabled="busy || a.operations === null" @click="restore(a)">Restore</Button>
            <Button look="danger" :disabled="busy" @click="forget(a)">Delete</Button>
          </div>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.st-wrap { position: absolute; inset: 0; overflow-y: auto; scrollbar-gutter: stable; }
.st-column { padding: var(--win-pad); }
.head { display: flex; align-items: center; gap: var(--s3); }
h1 { font-size: var(--display); font-weight: 700; margin: 0; letter-spacing: -0.01em; }
.st-sec { margin-top: var(--s6); display: flex; flex-direction: column; gap: var(--s3); }
.st-sec h2 { font-size: var(--body); font-weight: 600; margin: 0; }
.st-switch { display: flex; align-items: flex-start; gap: var(--s2); font-size: var(--small); max-width: 66ch; }
.st-switch em { display: block; font-style: normal; font-size: var(--fine); color: var(--text-faint); line-height: 1.5; margin-top: 3px; }
.st-why { font-size: var(--small); color: var(--text-quiet); margin: 0; max-width: 70ch; line-height: 1.5; }
.st-do { display: flex; gap: var(--s2); }
.st-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1.2fr) 70px auto;
  gap: var(--s3);
  align-items: center;
  padding: var(--s2) 0;
  border-bottom: var(--bw) solid var(--edge);
}
.st-who { display: flex; flex-direction: column; min-width: 0; }
.st-when { font-size: var(--small); font-weight: 600; }
.st-name { font-size: var(--fine); color: var(--text-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.st-holds { font-size: var(--fine); color: var(--text-quiet); }
.st-bad { color: var(--bad); }
.st-size { font-size: var(--fine); color: var(--text-faint); text-align: right; }
.st-act { display: flex; gap: var(--s2); }
</style>
