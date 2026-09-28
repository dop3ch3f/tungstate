<!-- Every sync, a way to make one, and what they have done. How it works is
     shown until there is a sync to show instead. -->
<script setup lang="ts">
import { computed, onMounted } from "vue";
import { ago } from "../../lib/format";
import { useSync } from "../../state/useSync";
import type { PastSync } from "../../engine/types";
import { exactly, way } from "../../lib/syncwords";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import Steps from "../../ui/Steps.vue";
import SyncRuns from "./SyncRuns.vue";
import SyncUndoneNotice from "./SyncUndoneNotice.vue";

const s = useSync();
/** When each sync last ran; `past` arrives newest first. */
const lastRun = computed(() => {
  const seen = new Map<string, number>();
  for (const run of s.past.value) if (!seen.has(run.sync)) seen.set(run.sync, run.applied_at);
  return seen;
});

const STEPS = [
  { art: "folder", title: "Pick two folders or more", line: "On this Mac, a drive, or a place on the NAS." },
  { art: "sync", title: "Say which way", line: "Send, bring in, or both ways. Deletions too, or only add." },
  { art: "in-step", title: "Preview, then run", line: "Every run can be put back on every folder, from here." },
] as const;

onMounted(() => void s.load());

async function putBack(run: PastSync) {
  if (s.busy.value != null) return;
  const answer = await ask({
    title: `Put back this run of ${run.sync}?`,
    why: "Copies it made are set aside again, and anything it set aside goes back where it was, on every folder.",
    choices: [
      { id: "cancel", label: "Cancel" },
      { id: "go", label: "Put back", look: "primary" },
    ],
  });
  if (answer.id === "go") await s.putBack(run.sync, run.plan);
}
</script>

<template>
  <div class="ss">
    <p class="ss-intro">
      Folders kept in step: on this Mac, on a drive, or on the NAS. Every run
      is shown before it happens, and can be put back afterwards.
    </p>

    <Steps v-if="s.loaded.value && !s.list.value.length && !s.past.value.length" :steps="[...STEPS]" />

    <div class="ss-new">
      <Button look="primary" @click="s.making()">New sync…</Button>
    </div>

    <ul class="ss-list" v-if="s.list.value.length">
      <li v-for="sync in s.list.value" :key="sync.name">
        <button class="ss-row" @click="s.open(sync)">
          <span class="ss-name"><b>{{ sync.name }}</b><span class="ss-chip" v-if="s.isHeld(sync.name)">waiting for you</span></span>
          <span class="ss-way">{{ way(sync) }} · {{ exactly(sync) }}</span>
          <span class="ss-members">{{ sync.members.map((m) => m.name).join(" · ") }}</span>
          <span class="ss-launch" v-if="sync.launch === 'continuous'">kept in step</span>
          <span class="ss-launch" v-else-if="sync.launch !== 'no'">runs when the app opens</span>
          <span class="ss-go" aria-hidden="true">›</span>
          <span class="ss-last">{{ lastRun.has(sync.name) ? `ran ${ago(lastRun.get(sync.name)!)}` : "not run yet" }}</span>
        </button>
      </li>
    </ul>

    <SyncUndoneNotice />
    <Notice tone="bad" v-if="s.problem.value">{{ s.problem.value }}</Notice>

    <SyncRuns v-if="s.loaded.value && s.past.value.length" title="Recent runs" :runs="s.past.value" :busy="s.busy.value" @put-back="putBack" />
  </div>
</template>

<style scoped>
.ss { display: flex; flex-direction: column; gap: var(--s4); }
.ss-intro { font-size: var(--body); color: var(--text-quiet); margin: 0; max-width: 70ch; line-height: 1.55; }
.ss-new { display: flex; }
.ss-list { list-style: none; margin: 0; padding: 0; border-top: var(--bw) solid var(--rule); }
.ss-list li { border-bottom: var(--bw) solid var(--rule); }
.ss-row {
  display: grid;
  grid-template-columns: 160px minmax(0, 1fr) auto 14px;
  grid-template-areas: "name way launch go" "name members last go";
  column-gap: var(--s3);
  row-gap: 2px;
  align-items: baseline;
  width: 100%;
  padding: 9px var(--s2);
  font: inherit;
  text-align: left;
  color: inherit;
  background: none;
  border: none;
  cursor: pointer;
}
.ss-row:hover { background: var(--surface-hover); }
.ss-go { grid-area: go; align-self: center; justify-self: end; font-size: var(--body); color: var(--text-faint); }
.ss-row:hover .ss-go { color: var(--text); }
.ss-name { grid-area: name; font-size: var(--small); }
.ss-way { grid-area: way; font-size: var(--small); color: var(--text-quiet); }
.ss-members { grid-area: members; font-size: var(--fine); color: var(--text-faint); }
.ss-launch { grid-area: launch; font-size: var(--fine); color: var(--text-faint); }
.ss-chip { margin-left: var(--s2); font-size: var(--fine); font-weight: 700; color: var(--warn-ink); background: var(--warn-bar); border-radius: var(--radius); padding: 0 6px; }
.ss-last { grid-area: last; justify-self: end; font-size: var(--fine); color: var(--text-faint); }
.ss-launch { justify-self: end; }
</style>
