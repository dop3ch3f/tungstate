<!-- A Copy or Move from the clipboard, running: how far, and three plain
     counts. Matches docs/mobile/approved-files/moving.png. "Keep browsing"
     leaves it running behind a bar above the tabs; Pause and Stop both
     finish the file in hand first, so nothing is left half sent. -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { bytes, duration, plural } from "../../lib/format";
import { useTransfer } from "../../state/useTransfer";
import { nameOf } from "../lib/paths";
import { tally, unfinished } from "../lib/progress";
import { useClipboard } from "../state/useClipboard";
import PButton from "../ui/PButton.vue";
import PScene from "../ui/PScene.vue";

defineEmits<{ hide: [] }>();

const board = useClipboard();
const t = useTransfer();
const run = computed(() => board.started.value!);

const ended = computed(() => t.finished.value.find((r) => r.job.id === run.value.job) ?? null);
const live = computed(() => !ended.value && t.current.value?.id === run.value.job);
/** Waiting behind another transfer: the engine runs one at a time. */
const queued = computed(() => !ended.value && !live.value);
// A copy once it has ended: the ledger hands over the very array it was
// filling, and an unchanged reference would leave the counts from mid-run.
const rows = computed(() => (ended.value ? [...ended.value.rows] : live.value ? t.rows.value : []));
const n = computed(() => tally(rows.value));
const moving = computed(() => run.value.op === "move");
/** Pause was pressed: the same stop, but the ending offers the rest. */
const paused = ref(false);

// The time left is the pace so far, re-read each second.
const now = ref(Date.now());
const since = ref(Date.now());
let timer: ReturnType<typeof setInterval> | undefined;
onMounted(() => (timer = setInterval(() => (now.value = Date.now()), 1000)));
onUnmounted(() => clearInterval(timer));
watch(live, (on) => on && (since.value = Date.now()));
const left = computed(() => {
  const elapsed = (now.value - since.value) / 1000;
  if (!live.value || n.value.bytes <= 0 || elapsed < 5) return null;
  return ((n.value.total - n.value.bytes) / n.value.bytes) * elapsed;
});

const BLOCKS = 20;
const filled = computed(() => (n.value.total ? Math.round((n.value.bytes / n.value.total) * BLOCKS) : 0));

const title = computed(() => {
  const verb = moving.value ? "Moving" : "Copying";
  if (!ended.value) return `${verb} ${run.value.what}`;
  const s = ended.value.summary;
  if (!s) return `${verb} stopped`;
  if (s.cancelled) return paused.value ? "Paused" : "Stopped";
  return n.value.failed ? `${plural(n.value.checked, "item")} ${moving.value ? "moved" : "copied"}` : `${run.value.what} ${moving.value ? "moved" : "copied"}`;
});

const rest = computed(() => (ended.value?.summary?.cancelled ? unfinished(run.value.paths, rows.value, nameOf) : []));

async function again() {
  paused.value = false;
  try {
    await board.again(rest.value);
  } catch (e) {
    t.problem.value = String(e);
  }
}

function pause() {
  paused.value = true;
  void t.cancel();
}
</script>

<template>
  <section class="screen">
    <button v-if="!ended" class="keep" @click="$emit('hide')">‹ Keep browsing</button>
    <div class="body">
      <h1>{{ title }}</h1>
      <p class="quiet lead">To {{ run.to }}</p>

      <template v-if="!ended">
        <PScene of="send" class="art" />
        <div class="centre">
          <div class="big">{{ bytes(n.bytes) }}</div>
          <div class="quiet">{{ queued ? "Waiting for the transfer before it to finish" : left === null ? `of ${bytes(n.total)}` : `of ${bytes(n.total)} · about ${duration(left)} left` }}</div>
        </div>
        <div class="blocks" aria-hidden="true"><i v-for="b in BLOCKS" :key="b" :class="{ on: b <= filled }"></i></div>
      </template>

      <div class="stats">
        <div><b>{{ n.sent.toLocaleString() }}</b><span>sent</span></div>
        <div><b>{{ n.checked.toLocaleString() }}</b><span>checked</span></div>
        <div v-if="moving"><b>{{ n.checked.toLocaleString() }}</b><span>off the phone</span></div>
        <div v-else-if="n.failed"><b>{{ n.failed.toLocaleString() }}</b><span>failed</span></div>
      </div>

      <p v-if="!ended" class="quiet centre">
        {{ moving ? "Each original is removed only after its copy is read back and matches." : "Each copy is read back and checked." }}
        Keep Tungstate open until this finishes.
      </p>
      <template v-else>
        <PScene v-if="n.checked && !n.failed && !ended.summary?.cancelled" of="done" class="art" />
        <p v-if="ended.problem" class="bad">{{ ended.problem }}</p>
        <p v-if="ended.summary?.cancelled" class="quiet">
          {{ rest.length ? `${plural(rest.length, "item")} not ${moving ? "moved" : "copied"} yet ${rest.length === 1 ? "stays where it was" : "stay where they were"}.` : "Everything chosen was done before it stopped." }}
        </p>
        <p v-if="n.failed" class="bad">{{ plural(n.failed, "file") }} could not be {{ moving ? "moved, and stay" : "copied, and stay" }} where they were.</p>
        <ul v-if="ended.summary?.failures.length" class="fails">
          <li v-for="f in ended.summary.failures.slice(0, 3)" :key="f.path">{{ f.path }}: {{ f.reason }}</li>
        </ul>
      </template>
      <p v-if="t.problem.value" class="bad">{{ t.problem.value }}</p>
    </div>

    <div class="act">
      <div v-if="!ended" class="pair">
        <PButton quiet :busy="t.stopping.value && paused" :disabled="t.stopping.value" @press="pause">{{ t.stopping.value && paused ? "Pausing…" : "Pause" }}</PButton>
        <PButton quiet :busy="t.stopping.value && !paused" :disabled="t.stopping.value" @press="t.cancel()">{{ t.stopping.value && !paused ? "Stopping…" : "Stop" }}</PButton>
      </div>
      <div v-else class="col">
        <PButton v-if="rest.length" @press="again">{{ moving ? "Move" : "Copy" }} the rest</PButton>
        <PButton :quiet="rest.length > 0" @press="board.done()">Done</PButton>
      </div>
    </div>
  </section>
</template>

<style scoped>
.screen { flex: 1; min-height: 0; display: flex; flex-direction: column; }
.keep { align-self: flex-start; min-height: 44px; margin: 0 20px; padding: 0; font: inherit; font-size: 17px; font-weight: 700; color: var(--text); background: none; border: 0; }
.body { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 18px; padding: 4px 20px 16px; }
h1 { margin: 0; font-size: 32px; line-height: 1.08; font-weight: 800; letter-spacing: -0.6px; }
.lead { margin: -10px 0 0; }
.quiet { font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.centre { text-align: center; }
.centre .quiet { margin-top: 8px; }
.big { font-size: 64px; font-weight: 800; letter-spacing: -1.5px; line-height: 0.95; }
.art { margin-top: 6px; }
.blocks { display: grid; grid-template-columns: repeat(20, 1fr); gap: 3px; height: 34px; }
.blocks i { border: 2px solid var(--edge); border-radius: 3px; background: var(--panel); }
.blocks i.on { background: var(--tint-drain); }
.stats { display: grid; grid-auto-flow: column; grid-auto-columns: 1fr; overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.stats div { padding: 12px; border-right: 2px solid var(--edge); }
.stats div:last-child { border-right: 0; }
.stats b { display: block; font-size: 22px; font-weight: 800; }
.stats span { font-size: 14px; color: var(--text-quiet); }
.bad { margin: 0; font-size: 15px; font-weight: 700; color: var(--bad); }
.fails { margin: 0; padding-left: 18px; font-size: 14px; line-height: 1.4; color: var(--text-quiet); }
.act { flex: none; padding: 10px 20px 12px; }
.pair { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
.col { display: flex; flex-direction: column; gap: 12px; }
</style>
