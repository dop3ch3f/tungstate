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
        <button class="door door-folder" @click="nav.go('folder')">
          <span class="stat-row">
            <span class="stat num">{{ governed }}</span>
            <span class="stat-of">{{ governed === 1 ? "folder" : "folders" }}</span>
          </span>
          <span class="door-name">Tidy a folder</span>
          <span class="door-what">
            See how a folder is filed now, beside what every other way of filing
            would do to it. Nothing moves until you say so.
          </span>
        </button>

        <button class="door door-drain" @click="nav.go('drain')">
          <span class="stat-row">
            <span class="stat num">{{ linked }}</span>
            <span class="stat-of">{{ linked === 1 ? "saved pair" : "saved pairs" }}</span>
          </span>
          <span class="door-name">Move files to another machine</span>
          <span class="door-what">
            Copy one, check it arrived intact, then remove the original. Close
            the lid part-way through and nothing is lost.
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
.home { position: absolute; inset: 0; display: flex; justify-content: center; align-items: center; overflow-y: auto; }
.column { width: min(820px, calc(100vw - var(--s6) * 2)); padding: var(--s6) 0; }
.unfinished { margin-bottom: var(--s4); }

.doors { display: grid; grid-template-columns: 1fr 1fr; gap: var(--s4); }
/* Each door is painted in the colour of the half it opens, so the colour you
   click is the colour you arrive in. */
.door {
  display: flex;
  flex-direction: column;
  gap: var(--s2);
  min-height: 240px;
  text-align: left;
  font: inherit;
  color: inherit;
  border: 1px solid var(--edge);
  border-radius: var(--radius-xl);
  padding: var(--s5) var(--s5) var(--s5);
  box-shadow: var(--glass-lip), var(--drop);
  cursor: pointer;
  transition: transform var(--slow) var(--ease), filter var(--slow) var(--ease);
}
.door:hover { transform: translateY(-2px); filter: brightness(1.08); }
.door-folder { background: var(--field-folder); }
.door-drain { background: var(--field-drain); }
.stat-row { display: flex; align-items: baseline; gap: var(--s2); flex: 1; }
.stat { font-size: var(--hero); font-weight: 700; line-height: 1; letter-spacing: -0.02em; }
.stat-of { font-size: var(--body); color: var(--text-quiet); }
.door-name { font-size: var(--title); font-weight: 700; letter-spacing: -0.01em; }
.door-what { font-size: var(--small); color: var(--text-quiet); line-height: 1.55; }

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
