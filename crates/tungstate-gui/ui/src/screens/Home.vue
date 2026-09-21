<!-- The window opens here, and this screen belongs to neither half.
     Two doors, equal weight, each showing what it actually has rather than
     what it is for. Anything interrupted sits above both, because unfinished
     work is the one thing that should be seen before a choice is made. -->
<script setup lang="ts">
import { onMounted, ref, shallowRef } from "vue";
import { useNav } from "../nav";
import { folders, links, transfers, history } from "../engine/commands";
import { when } from "../lib/format";
import type { InterruptedRun, Op } from "../engine/types";
import Button from "../ui/Button.vue";
import Notice from "../ui/Notice.vue";

const nav = useNav();
const governed = ref(0);
const linked = ref(0);
const stranded = shallowRef<InterruptedRun[]>([]);
const latest = shallowRef<Op | null>(null);
const problem = ref<string | null>(null);

onMounted(async () => {
  try {
    const [f, l, i, r] = await Promise.all([
      folders.governed(),
      links.list(),
      transfers.interrupted(),
      history.recent(),
    ]);
    governed.value = f.length;
    linked.value = l.length;
    stranded.value = i;
    latest.value = r[0] ?? null;
  } catch (e) {
    problem.value = String(e);
  }
});
</script>

<template>
  <div class="home">
    <div class="column">
      <Notice tone="bad" v-if="problem">{{ problem }}</Notice>

      <!-- Above both doors, and it does not steer you into one. The old
           window jumped straight to Transfers when it found one of these,
           which was right when Transfers was the only default there was. -->
      <Notice tone="hold" v-if="stranded.length" class="unfinished">
        {{ stranded.length === 1 ? "A transfer" : `${stranded.length} transfers` }}
        stopped part-way and can be picked up.
        <Button look="link" @click="nav.go('drain')">Look at it</Button>
      </Notice>

      <div class="doors">
        <button class="door" @click="nav.go('folder')">
          <span class="door-name">Tidy a folder</span>
          <span class="door-what">
            See how a folder is filed now, beside what every other way of filing
            would do to it. Nothing moves until you say so.
          </span>
          <span class="door-has">
            {{ governed === 0 ? "none yet" : `${governed} folder${governed === 1 ? "" : "s"}` }}
          </span>
        </button>

        <button class="door" @click="nav.go('drain')">
          <span class="door-name">Move files to another machine</span>
          <span class="door-what">
            Copy one, check it arrived intact, then remove the original. Close
            the lid part-way through and nothing is lost.
          </span>
          <span class="door-has">
            {{ linked === 0 ? "none yet" : `${linked} saved pair${linked === 1 ? "" : "s"}` }}
          </span>
        </button>
      </div>

      <p class="last" v-if="latest">
        Last activity: {{ latest.kind }}, {{ when(latest.started_at) }}.
        <Button look="link" @click="nav.go('history')">Everything that has happened</Button>
      </p>
    </div>
  </div>
</template>

<style scoped>
.home { position: absolute; inset: 0; display: flex; justify-content: center; overflow-y: auto; }
.column { width: min(760px, calc(100vw - var(--s6) * 2)); padding: 96px 0 var(--s6); }
.unfinished { margin-bottom: var(--s4); }

.doors { display: grid; grid-template-columns: 1fr 1fr; gap: var(--s3); }
.door {
  display: flex;
  flex-direction: column;
  gap: var(--s2);
  text-align: left;
  font: inherit;
  color: inherit;
  background: none;
  border: 1px solid var(--edge);
  border-radius: var(--radius-lg);
  padding: var(--s5);
  cursor: pointer;
  transition: border-color var(--quick) var(--ease), background var(--quick) var(--ease);
}
.door:hover { border-color: var(--text-faint); background: var(--surface-hover); }
.door-name { font-size: var(--body); font-weight: 600; }
.door-what { font-size: var(--small); color: var(--text-quiet); line-height: 1.55; flex: 1; }
.door-has { font-size: var(--fine); color: var(--text-faint); }

.last {
  font-size: var(--small);
  color: var(--text-faint);
  margin: var(--s5) 0 0;
  display: flex;
  gap: var(--s3);
  align-items: baseline;
  flex-wrap: wrap;
}
</style>
