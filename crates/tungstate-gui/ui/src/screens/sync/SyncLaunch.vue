<!-- The syncs marked to run when the app opens, shown once, after the check
     for interrupted work. Quiet ones that are safe have already gone; these
     are the ones that asked, and one yes runs them all, one after another. -->
<script setup lang="ts">
import { computed, onMounted, shallowRef } from "vue";
import type { SyncPreview, SyncView } from "../../engine/types";
import { arrivingAcross, files, removingAcross } from "../../lib/counts";
import { refusal, way } from "../../lib/syncwords";
import { launchPreviews, useSync } from "../../state/useSync";
import { useNav } from "../../nav";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import Sheet from "../../ui/Sheet.vue";

const s = useSync();
const nav = useNav();
const asking = shallowRef<{ sync: SyncView; preview: SyncPreview }[]>([]);

onMounted(async () => {
  asking.value = await launchPreviews();
});

/** Whether anything here would remove a file, or needs a yes. Then looking
 *  comes first and running all of them is the second choice. */
const careful = computed(() =>
  asking.value.some((a) => removingAcross(a.preview) > 0 || a.preview.refusals.length || !a.preview.reversible),
);

async function runAll() {
  const all = asking.value;
  asking.value = [];
  for (const one of all) await s.runPreviewed(one.sync.name, one.preview, true);
}

function lookAt(sync: SyncView) {
  asking.value = [];
  nav.go("sync");
  s.open(sync);
  void s.look();
}
</script>

<template>
  <Sheet v-if="asking.length" wide of="sync" title="Syncs to run now the app is open" @dismiss="asking = []">
    <!-- Its own section's colours, wherever in the window it opens: the
         sheet is teleported to the frame, so the scope is set in here. -->
    <div data-half="sync" class="la">
    <ul class="la-list">
      <li v-for="one in asking" :key="one.sync.name" class="la-one">
        <b>{{ one.sync.name }}</b>
        <span class="la-way">{{ way(one.sync) }}</span>
        <span class="la-what">
          {{ files(arrivingAcross(one.preview)) }} to copy<template v-if="removingAcross(one.preview)">,
          {{ files(removingAcross(one.preview)) }} removed, and kept in the set-aside area</template><template v-if="one.preview.conflicts.length">,
          {{ one.preview.conflicts.length }} changed on more than one folder</template>
        </span>
        <Notice tone="hold" v-for="r in one.preview.refusals" :key="`${r.kind}${r.member}`">{{ refusal(r) }}</Notice>
        <Notice tone="bad" v-if="!one.preview.reversible">This one deletes outright and cannot be put back.</Notice>
        <Button v-if="asking.length > 1" look="link" @click="lookAt(one.sync)">Look at it first</Button>
      </li>
    </ul>
    <footer class="la-foot">
      <template v-if="careful">
        <Button look="primary" @click="lookAt(asking[0]!.sync)">Look at {{ asking.length === 1 ? "it" : asking[0]!.sync.name }} first</Button>
        <Button @click="runAll()">Run {{ asking.length === 1 ? "it" : `all ${asking.length}` }} anyway</Button>
      </template>
      <Button v-else look="primary" @click="runAll()">Run {{ asking.length === 1 ? "it" : `all ${asking.length}` }}</Button>
      <Button look="link" @click="asking = []">Not now</Button>
    </footer>
    </div>
  </Sheet>
</template>

<style scoped>
.la { display: flex; flex-direction: column; }
.la-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--s3); }
.la-one { display: flex; flex-direction: column; gap: var(--s1); padding-bottom: var(--s3); border-bottom: var(--bw) solid var(--rule); font-size: var(--small); align-items: flex-start; }
.la-way { color: var(--text-quiet); }
.la-what { color: var(--text-faint); font-size: var(--fine); }
.la-foot { display: flex; gap: var(--s3); align-items: center; margin-top: var(--s4); }
</style>
