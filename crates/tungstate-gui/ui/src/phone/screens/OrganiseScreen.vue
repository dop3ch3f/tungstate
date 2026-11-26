<!-- Organise a folder on this phone: choose it, choose how to sort it (only
     the first time; the choice is kept in the folder), see where every file
     would go, then organise. Only files inside the chosen folder move, and
     Put back undoes the lot. The desktop's own rules, preview and tidy,
     drawn for a phone. Android only for now: an iPhone app cannot see these
     folders. -->
<script setup lang="ts">
import { computed, onMounted, ref, shallowRef } from "vue";
import { folders } from "../../engine/commands";
import type { Outcome, PastRun, PreviewView, TidyDone } from "../../engine/types";
import { ago, bytes, plural } from "../../lib/format";
import { useBack } from "../lib/back";
import { destinations, layoutFor } from "../lib/organise";
import { nameOf } from "../lib/paths";
import PButton from "../ui/PButton.vue";
import PPlaces from "../ui/PPlaces.vue";
import PScreen from "../ui/PScreen.vue";
import PTrouble from "../ui/PTrouble.vue";

type Phase = "start" | "layouts" | "preview" | "running" | "done";
const phase = ref<Phase>("start");
const root = ref<string | null>(null);
const outcomes = shallowRef<Outcome[]>([]);
const layout = ref<string | null>(null);
const preview = shallowRef<PreviewView | null>(null);
const done = shallowRef<TidyDone | null>(null);
const past = shallowRef<PastRun[]>([]);
const busy = ref(false);
const problem = ref<string | null>(null);
const said = ref<string | null>(null);
/** The plan the last organise made, for Put back. */
const plan = ref<number | null>(null);

onMounted(() => void loadPast());
async function loadPast() {
  try {
    past.value = await folders.past();
  } catch {
    past.value = [];
  }
}
const last = computed(() => past.value.find((r) => r.undoable && r.root.startsWith("/storage/")) ?? null);
const name = computed(() => (root.value ? nameOf(root.value) : ""));

useBack(() => {
  if (phase.value === "start" || phase.value === "running") return false;
  phase.value = phase.value === "preview" && outcomes.value.length ? "layouts" : "start";
  return true;
});

async function act(job: () => Promise<void>) {
  busy.value = true;
  problem.value = null;
  try {
    await job();
  } catch (e) {
    problem.value = String(e);
  } finally {
    busy.value = false;
  }
}

/** A folder with rules goes straight to its preview; one without is asked how. */
const look = () =>
  act(async () => {
    const at = root.value!;
    said.value = null;
    outcomes.value = [];
    if (await folders.hasRules(at)) return showPreview();
    // Every starting layout tried on paper, nothing written yet.
    outcomes.value = (await folders.compare(at)).filter((o) => o.loads && o.settles && o.summary);
    const suggested = layoutFor(at);
    layout.value = outcomes.value.find((o) => o.name === suggested)?.name ?? outcomes.value[0]?.name ?? null;
    phase.value = "layouts";
  });

/** Keeps the chosen layout in the folder, as its rules, then previews it. */
const useLayout = () =>
  act(async () => {
    const at = root.value!;
    // Already looked after (an earlier visit): that is fine.
    await folders.govern(at).catch(() => {});
    await folders.giveRules(at, layout.value!);
    await showPreview();
  });

async function showPreview() {
  preview.value = await folders.preview(root.value!);
  phase.value = "preview";
}

const run = () =>
  act(async () => {
    phase.value = "running";
    try {
      done.value = await folders.tidy(root.value!);
      plan.value = (await folders.preview(root.value!).catch(() => null))?.undoable ?? null;
      phase.value = "done";
      await loadPast();
    } catch (e) {
      phase.value = "preview";
      throw e;
    }
  });

const putBack = (at: string, id: number) =>
  act(async () => {
    const back = await folders.putBack(at, id);
    said.value = `Put ${plural(back.files, "file")} back where they were.`;
    phase.value = "start";
    await loadPast();
  });

const where = computed(() => (preview.value ? destinations(preview.value.moves) : []));
const SHOWN = 6;
const exampleOf = (o: Outcome) => (o.example ? `${nameOf(o.example.from)} → ${o.example.to.split("/").slice(0, -1).join(" › ")}` : "");
</script>

<template>
  <PScreen v-if="phase === 'start'" title="Organise" :tile="{ art: 'organise', tint: 'var(--tint-folder)', light: true }">
    <PTrouble v-if="problem" :error="problem" />
    <p v-if="said" class="line good">{{ said }}</p>
    <h2>Folder</h2>
    <PPlaces v-model="root" wanted="organise" />
    <p class="line note">Loose files go into folders by year, app, kind and type. You see where each one goes before anything moves.</p>
    <template v-if="last">
      <h2>Last time</h2>
      <div class="list">
        <div class="row">
          <span class="text"><b>{{ last.name }} · {{ plural(last.files, "file") }}</b><span class="quiet">{{ ago(last.applied_at) }}</span></span>
          <PButton quiet :busy="busy" @press="putBack(last.root, last.plan)">Put back</PButton>
        </div>
      </div>
    </template>
    <template #act>
      <PButton tint="var(--tint-folder)" :busy="busy" :disabled="!root" @press="look">See what would move</PButton>
    </template>
  </PScreen>

  <PScreen v-else-if="phase === 'layouts'" title="How to sort it" :back="name" @back="phase = 'start'">
    <p class="line">Each way, tried on {{ name }} without moving anything.</p>
    <PTrouble v-if="problem" :error="problem" />
    <div class="list">
      <button v-for="o in outcomes" :key="o.name" class="pick" :class="{ on: layout === o.name }" @click="layout = o.name">
        <span class="radio" :class="{ on: layout === o.name }" aria-hidden="true"></span>
        <span class="text">
          <b>{{ o.summary }}</b>
          <span class="quiet">{{ o.files ? `${plural(o.files, "file")} of ${o.of.toLocaleString()} would move` : "Nothing would move" }}</span>
          <span v-if="o.example" class="quiet">{{ exampleOf(o) }}</span>
        </span>
      </button>
    </div>
    <p v-if="!outcomes.length" class="line">There's nothing here to sort.</p>
    <template #act>
      <PButton tint="var(--tint-folder)" :busy="busy" :disabled="!layout" @press="useLayout">See where they go</PButton>
    </template>
  </PScreen>

  <PScreen v-else-if="phase === 'preview' && preview" :title="name" :back="outcomes.length ? 'How to sort it' : 'Organise'" @back="phase = outcomes.length ? 'layouts' : 'start'">
    <PTrouble v-if="problem" :error="problem" />
    <template v-if="preview.files">
      <p class="line">{{ plural(preview.files, "loose file") }}, {{ bytes(preview.bytes) }}, would go into these folders.</p>
      <div class="list">
        <div v-for="d in where.slice(0, SHOWN)" :key="d.folder" class="row tight">
          <span class="text"><b>{{ d.folder }}</b></span>
          <span class="num">{{ d.files.toLocaleString() }}</span>
        </div>
        <div v-if="where.length > SHOWN" class="row tight"><span class="text"><b>{{ plural(where.length - SHOWN, "more folder") }}</b></span></div>
      </div>
      <p v-if="preview.waiting" class="line note">{{ plural(preview.waiting, "file") }} changed too recently and will wait for a later run.</p>
      <p class="line note">Only files inside {{ name }} move. Put back undoes all of it.</p>
    </template>
    <p v-else class="line">{{ name }} is already organised. Nothing would move.</p>
    <PTrouble v-if="!preview.settles" error="These rules would keep moving the same files, so Tungstate won't run them." />
    <template #act>
      <PButton tint="var(--tint-folder)" :busy="busy" :disabled="!preview.files || !preview.settles" @press="run">Organise {{ plural(preview.files, "file") }}</PButton>
    </template>
  </PScreen>

  <PScreen v-else-if="phase === 'running'" :title="`Organising ${name}`">
    <p class="line">Moving each file into its folder. Leave the app open until it's done.</p>
  </PScreen>

  <PScreen v-else title="Organised" back="Organise" @back="phase = 'start'">
    <p class="line good">{{ done?.already_tidy ? `${name} was already organised.` : `Moved ${plural(done?.moved ?? 0, "file")} into folders in ${name}.` }}</p>
    <p v-if="done?.skipped" class="line">{{ plural(done.skipped, "file") }} changed while Tungstate looked, so it was left where it was.</p>
    <PTrouble v-if="done?.failed" :error="`${plural(done.failed, 'file')} could not be moved.`" />
    <template #act>
      <div class="pair">
        <PButton v-if="plan !== null && root" quiet :busy="busy" @press="putBack(root, plan!)">Put back</PButton>
        <PButton tint="var(--tint-folder)" @press="phase = 'start'">Done</PButton>
      </div>
    </template>
  </PScreen>
</template>

<style scoped>
h2 { margin: 18px 0 8px; font-size: 15px; font-weight: 800; }
h2:first-of-type { margin-top: 0; }
.line { margin: 0 2px 12px; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.note { margin-top: 12px; }
.good { color: var(--ok); font-weight: 700; }
.list { overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.row { display: flex; align-items: center; gap: 12px; padding: 12px 14px; border-top: 2px solid var(--edge); }
.row:first-child { border-top: 0; }
.row.tight { min-height: 52px; padding: 8px 14px; }
.text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.text b { font-size: 15px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.quiet { font-size: 14px; color: var(--text-quiet); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.num { flex: none; font-size: 16px; font-weight: 800; }
.pick .text b { white-space: normal; }
.pick { display: flex; align-items: flex-start; gap: 12px; width: 100%; padding: 12px 14px; font: inherit; text-align: left; color: var(--text); background: var(--panel); border: 0; border-top: 2px solid var(--edge); }
.pick:first-child { border-top: 0; }
.pick.on { background: var(--surface-hover); }
.radio { position: relative; flex: none; width: 26px; height: 26px; margin-top: 2px; background: var(--panel); border: 2px solid var(--edge); border-radius: 99px; }
.radio.on::after { content: ""; position: absolute; inset: 4px; border-radius: 99px; background: var(--tint-folder); }
.pair { display: flex; gap: 12px; }
.pair > * { flex: 1; }
</style>
