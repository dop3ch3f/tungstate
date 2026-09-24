<!-- Point at something. Any folder, governed or not, or a connection: finding
     duplicates has nothing to do with rules. -->
<script setup lang="ts">
import { onMounted, ref, shallowRef } from "vue";
import { useDupes } from "../../state/useDupes";
import { dupes, folders, transfers } from "../../engine/commands";
import { files, ran, undidDuplicates } from "../../lib/counts";
import { shortPath } from "../../lib/format";
import type { PastRun, Place } from "../../engine/types";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import PastRuns from "../../ui/PastRuns.vue";
import Steps from "../../ui/Steps.vue";

const d = useDupes();
const places = shallowRef<Place[]>([]);
const chosen = ref("");
const past = shallowRef<PastRun[]>([]);
const loaded = ref(false);
const busy = ref<number | null>(null);
const said = ref<string | null>(null);
const trouble = ref<string | null>(null);

async function loadPast() {
  try {
    past.value = await dupes.past();
  } catch (e) {
    trouble.value = String(e);
  } finally {
    loaded.value = true;
  }
}

async function putBack(run: PastRun) {
  const answer = await ask({
    title: `Put back ${files(ran(run))} in ${run.name}?`,
    why: "Each copy goes back where it was before it was set aside.",
    choices: [
      { id: "cancel", label: "Cancel" },
      { id: "go", label: "Put back", look: "primary" },
    ],
  });
  if (answer.id !== "go") return;
  busy.value = run.plan;
  said.value = trouble.value = null;
  try {
    const back = await dupes.putBack(run.root, run.plan);
    said.value = `Put ${files(undidDuplicates(back))} back in ${run.name}.`;
  } catch (e) {
    trouble.value = String(e);
  } finally {
    busy.value = null;
    await loadPast();
  }
}

const STEPS = [
  { art: "folder", title: "Point at a folder", line: "Or somewhere saved, like the NAS. No rules are needed." },
  { art: "nothing-found", title: "Look at each group", line: "With the picture in view, before anything is ticked." },
  { art: "dupes", title: "Set the extras aside", line: "They stay inside the folder and can be put back from here." },
] as const;

onMounted(async () => {
  void d.loadRecent();
  void loadPast();
  try {
    places.value = await transfers.places();
  } catch {
    // The list is a convenience; the picker still works without it.
  }
});

async function point() {
  const picked = await folders.pick();
  if (picked) await d.look(picked);
}
</script>

<template>
  <div class="dz-column">
    <p class="dz-intro">
      Files that are the same file, whatever they are called. Nothing is read
      in full unless it has to be, and nothing moves until you say so.
    </p>

    <label class="dz-also">
      <input type="checkbox" v-model="d.alsoSimilar.value" />
      <span>
        Also find files that are nearly the same
        <em>a photo re-exported smaller, a video re-encoded, one song at two
        bitrates. This opens every picture and video, so it takes longer the
        first time and is quick after that.</em>
      </span>
    </label>

    <Steps v-if="loaded && !past.length" :steps="[...STEPS]" />

    <div class="dz-pick">
      <Button look="primary" @click="point()">Point at a folder…</Button>
      <label class="dz-go">
        <span class="dz-lbl">or somewhere saved</span>
        <select v-model="chosen" @change="chosen && d.look(chosen)">
          <option value="">Go to…</option>
          <option v-for="place in places" :key="place.path" :value="place.path">
            {{ place.label }}
          </option>
        </select>
      </label>
    </div>

    <section class="dz-again" v-if="d.recent.value.length">
      <h2>Looked at before</h2>
      <button
        v-for="place in d.recent.value"
        :key="place"
        class="dz-recent"
        @click="d.look(place)"
      >
        <span class="path">{{ shortPath(place) }}</span>
      </button>
    </section>

    <Notice tone="bad" v-if="d.problem.value">{{ d.problem.value }}</Notice>
    <Notice v-if="said">{{ said }}</Notice>
    <Notice tone="bad" v-if="trouble">{{ trouble }}</Notice>

    <PastRuns
      v-if="loaded"
      of="dupes"
      title="Recent clean-ups"
      verb="Set aside"
      empty="Clean-ups you run are listed here, and each one can be put back."
      :runs="past"
      :busy="busy"
      @put-back="putBack"
    />
  </div>
</template>

<style scoped>
.dz-column { display: flex; flex-direction: column; gap: var(--s4); }
.dz-intro { font-size: var(--body); color: var(--text-quiet); margin: 0; max-width: 70ch; line-height: 1.55; }
.dz-also { display: flex; align-items: flex-start; gap: var(--s2); font-size: var(--small); max-width: 62ch; }
.dz-also em { display: block; font-style: normal; font-size: var(--fine); color: var(--text-faint); line-height: 1.5; margin-top: 2px; }
.dz-pick { display: flex; align-items: flex-end; gap: var(--s4); flex-wrap: wrap; }
.dz-go { display: flex; flex-direction: column; gap: 5px; }
.dz-lbl { font-size: var(--fine); color: var(--text-faint); }
.dz-again h2 { font-size: var(--small); font-weight: 700; margin: 0 0 var(--s2); text-transform: uppercase; letter-spacing: 0.04em; }
.dz-recent {
  display: block;
  width: 100%;
  margin: 0 0 2px;
  padding: 5px var(--s2);
  font: inherit;
  text-align: left;
  color: var(--text-quiet);
  background: none;
  border: none;
  border-radius: var(--radius);
  cursor: pointer;
}
.dz-recent:hover { color: var(--text); background: var(--surface-hover); }
</style>
