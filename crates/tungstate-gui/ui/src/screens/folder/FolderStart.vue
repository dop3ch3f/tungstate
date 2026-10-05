<!-- The front door. One button, and it does not require registering anything.
     That inversion is the point of the slice: `learn_folder` and
     `compare_folder` take a bare path, so somebody can be shown their own
     files before they have learned a word of this app's vocabulary. -->
<script setup lang="ts">
import { computed, onMounted, ref, shallowRef } from "vue";
import { folders } from "../../engine/commands";
import type { PastRun } from "../../engine/types";
import { files, putBack as putBackCount, ran } from "../../lib/counts";
import { ago, shortPath } from "../../lib/format";
import { useFolders } from "../../state/useFolders";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import Tile from "../../ui/Tile.vue";
import Notice from "../../ui/Notice.vue";
import PastRuns from "../../ui/PastRuns.vue";
import Steps from "../../ui/Steps.vue";

const f = useFolders();
const past = shallowRef<PastRun[]>([]);
const loaded = ref(false);
const busy = ref<number | null>(null);
const said = ref<string | null>(null);
const trouble = ref<string | null>(null);

async function loadPast() {
  try {
    past.value = await folders.past();
  } catch (e) {
    trouble.value = String(e);
  } finally {
    loaded.value = true;
  }
}

onMounted(() => {
  void f.listRegistered();
  void loadPast();
});

async function putBack(run: PastRun) {
  if (busy.value != null) return;
  const answer = await ask({
    title: `Put back ${files(ran(run))} in ${run.name}?`,
    why: "Each file goes back where it was before that tidy. Anything changed since is left alone and said so.",
    choices: [
      { id: "cancel", label: "Cancel" },
      { id: "go", label: "Put back", look: "primary" },
    ],
  });
  if (answer.id !== "go") return;
  busy.value = run.plan;
  said.value = trouble.value = null;
  try {
    const done = await folders.putBack(run.root, run.plan);
    said.value = `Put ${files(putBackCount(done))} back in ${run.name}.`;
  } catch (e) {
    trouble.value = String(e);
  } finally {
    busy.value = null;
    await loadPast();
  }
}

/** When each folder was last tidied; `past` arrives newest first. */
const lastTidy = computed(() => {
  const seen = new Map<string, number>();
  for (const run of past.value) if (!seen.has(run.root)) seen.set(run.root, run.applied_at);
  return seen;
});

const STEPS = [
  { art: "folder", title: "Point at a folder", line: "Its own shape is read first, so nothing is assumed about it." },
  { art: "history", title: "See every move", line: "Before and after side by side, with what stays put and why." },
  { art: "home", title: "Tidy up, or put it back", line: "Every tidy is listed here and can be undone." },
] as const;
</script>

<template>
  <div class="start">
    <div class="column">
      <div class="head"><Tile of="folder" :size="26" /><h1>Organize</h1></div>
      <p class="intro">
        See how a folder is filed now and what each way of filing would change.
        Nothing moves until you say so.
      </p>

      <Steps v-if="loaded && !past.length" class="steps" :steps="[...STEPS]" />

      <Button look="primary" @click="f.pick()">Choose a folder…</Button>

      <Notice tone="bad" v-if="f.problem.value">{{ f.problem.value }}</Notice>
      <Notice v-if="said">{{ said }}</Notice>
      <Notice tone="bad" v-if="trouble">{{ trouble }}</Notice>

      <section class="known" v-if="f.registered.value.length">
        <h2>Folders you have added</h2>
        <button
          v-for="folder in f.registered.value"
          :key="folder.root"
          class="known-row"
          :aria-busy="f.opening.value === folder.root || undefined"
          @click="folder.broken ? f.look(folder.root) : f.open(folder.root)"
        >
          <span class="known-name">{{ folder.name }}</span>
          <span class="path known-path" :title="folder.root">{{ shortPath(folder.root) }}</span>
          <span class="known-state" v-if="folder.broken">the rules file has a mistake</span>
          <span class="known-state dim" v-else-if="folder.has_rules === false">no rules yet</span>
          <span class="known-state dim" v-else-if="lastTidy.has(folder.root)">tidied {{ ago(lastTidy.get(folder.root)!) }}</span>
          <span class="known-state dim" v-else>not tidied yet</span>
        </button>
      </section>

      <PastRuns
        v-if="loaded && past.length"
        class="past"
        of="folder"
        title="Recent tidies"
        verb="Filed"
        empty="Tidies you run are listed here, and each one can be put back."
        :runs="past"
        :busy="busy"
        @put-back="putBack"
      />
    </div>
  </div>
</template>

<style scoped>
.head { display: flex; align-items: center; gap: var(--s3); }
.start { position: absolute; inset: 0; overflow-y: auto; scrollbar-gutter: stable; }
.column { padding: var(--win-pad); }

h1 { font-size: var(--display); font-weight: 700; margin: 0; letter-spacing: -0.01em; }
.intro {
  font-size: var(--body);
  line-height: 1.55;
  color: var(--text-quiet);
  margin: var(--s3) 0 var(--s5);
  max-width: 62ch;
}

.known { margin-top: 44px; }
.steps { margin-bottom: var(--s5); }
.past { margin-top: 44px; }
.known h2 { font-size: var(--small); font-weight: 600; margin: 0 0 var(--s2); }
.known-row {
  display: flex;
  align-items: baseline;
  gap: var(--s3);
  width: calc(100% + var(--s3) * 2);
  margin: 0 calc(var(--s3) * -1);
  padding: var(--s2) var(--s3);
  font: inherit;
  text-align: left;
  background: none;
  border: none;
  border-radius: var(--radius);
  color: inherit;
  cursor: pointer;
}
.known-row:hover { background: var(--surface-hover); }
.known-row[aria-busy="true"] { cursor: progress; background: var(--surface-hover); }
.known-row[aria-busy="true"] .known-name { animation: known-breathe 1.2s var(--ease) infinite; }
@keyframes known-breathe { 0%, 100% { opacity: 0.45 } 50% { opacity: 1 } }
.known-name { font-weight: 600; flex: none; }
.known-state { margin-left: auto; }
.known-path { color: var(--text-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.known-state { flex: none; font-size: var(--fine); color: var(--bad); }
.known-state.dim { color: var(--text-faint); }
</style>
