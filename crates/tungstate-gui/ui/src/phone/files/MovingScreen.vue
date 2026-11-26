<!-- A Copy or Move from the clipboard, running: how far overall, then every
     file with where it is, the way the desktop's Transfer lists them. The
     file being sent has its own moving bar, and the list keeps it in view
     until you scroll it yourself. "Keep browsing" leaves it running behind a
     bar above the tabs; Pause and Stop both finish the file in hand first,
     so nothing is left half sent. -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { bytes, duration, plural } from "../../lib/format";
import { useTransfer } from "../../state/useTransfer";
import { nameOf } from "../lib/paths";
import { isMoving, markOf, percentOf, rowWord, tally, unfinished } from "../lib/progress";
import { rowsInView, useScroll } from "../lib/virtual";
import { useClipboard } from "../state/useClipboard";
import PButton from "../ui/PButton.vue";
import PScene from "../ui/PScene.vue";
import PTrouble from "../ui/PTrouble.vue";

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

// ── the file list ───────────────────────────────────────────────────────
/** Every line is this tall, bar or word, so which lines are on screen is
 *  arithmetic and a 10,000-photo Free up scrolls as freely as 20 files. */
const LINE = 60;
const box = ref<HTMLElement | null>(null);
const scroll = useScroll(box);
const span = computed(() => rowsInView(scroll.top.value, scroll.height.value, LINE, rows.value.length));
const drawn = computed(() => rows.value.slice(span.value[0], span.value[1]));
// A lookup, not an interpolated class: `check-css.mjs` must see every mark.
const MARK = { waiting: "m-waiting", sending: "m-sending", checking: "m-checking", done: "m-done", same: "m-same", held: "m-held", failed: "m-failed" } as const;
const GLYPH = { waiting: "", sending: "›", checking: "›", done: "✓", same: "=", held: "–", failed: "✕" } as const;

/** Why each file failed. The engine says so in the run's summary, not on the
 *  file's own line, so a failed line looks its reason up here. */
const why = computed(() => new Map((ended.value?.summary?.failures ?? []).map((f) => [f.path, f.reason])));
const reasonOf = (r: { path: string; detail: string | null }) => r.detail ?? why.value.get(r.path) ?? null;
/** The reasons behind the failure bar's Details, a few of them. */
const reasons = computed(() => (ended.value?.summary?.failures ?? []).slice(0, 5).map((f) => `${nameOf(f.path) || f.path}: ${f.reason}`).join("\n"));

// Follows the file being sent, one line of what just finished above it,
// unless a finger has been on the list in the last few seconds.
let touched = 0;
const hands = () => (touched = Date.now());
const busyAt = computed(() => rows.value.findIndex((r) => isMoving(r.state)));
watch(busyAt, (at) => {
  if (at < 0 || !box.value || Date.now() - touched < 4000) return;
  box.value.scrollTop = Math.max(0, (at - 1) * LINE);
});

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
    <div class="top">
      <h1>{{ title }}</h1>
      <p class="quiet lead">To {{ run.to }}</p>

      <template v-if="!ended">
        <PScene of="send" :moving="live" class="art" />
        <div class="sum">
          <b class="big">{{ bytes(n.bytes) }}</b>
          <span class="quiet">{{ queued ? "Waiting for the transfer before it to finish" : left === null ? `of ${bytes(n.total)}` : `of ${bytes(n.total)} · about ${duration(left)} left` }}</span>
        </div>
        <div class="blocks" aria-hidden="true"><i v-for="b in BLOCKS" :key="b" :class="{ on: b <= filled, lead: live && b === filled + 1 }"></i></div>
      </template>
      <PScene v-else-if="n.checked && !n.failed && !ended.summary?.cancelled" of="done" class="art small" />

      <div class="counts">
        <span><b>{{ n.sent.toLocaleString() }}</b> of {{ n.count.toLocaleString() }} sent</span>
        <span><b>{{ n.checked.toLocaleString() }}</b> checked</span>
        <span v-if="n.failed" class="failed"><b>{{ n.failed.toLocaleString() }}</b> failed</span>
      </div>

      <template v-if="ended">
        <PTrouble v-if="ended.problem" :error="ended.problem" :where="run.to" />
        <PTrouble v-if="n.failed" :error="`${plural(n.failed, 'file')} could not be ${moving ? 'moved' : 'copied'}, and ${n.failed === 1 ? 'stays' : 'stay'} where ${n.failed === 1 ? 'it was' : 'they were'}.`" :detail="reasons || undefined" />
        <p v-if="ended.summary?.cancelled" class="quiet">
          {{ rest.length ? `${plural(rest.length, "item")} not ${moving ? "moved" : "copied"} yet ${rest.length === 1 ? "stays where it was" : "stay where they were"}.` : "Everything chosen was done before it stopped." }}
        </p>
      </template>
      <PTrouble v-if="t.problem.value" :error="t.problem.value" :where="run.to" />
    </div>

    <div v-show="rows.length" ref="box" class="files" @touchstart.passive="hands" @wheel.passive="hands">
      <div class="spacer" :style="{ height: `${rows.length * LINE}px` }">
        <ul class="slice" :style="{ transform: `translateY(${span[0] * LINE}px)` }">
          <li v-for="(r, i) in drawn" :key="`${span[0] + i}:${r.path}`" class="file" :class="{ going: isMoving(r.state) }">
            <span class="mark" :class="MARK[markOf(r.state)]" aria-hidden="true">{{ GLYPH[markOf(r.state)] }}</span>
            <span class="name">{{ nameOf(r.path) || r.path }}</span>
            <span class="size">{{ bytes(r.size) }}</span>
            <span v-if="isMoving(r.state)" class="meter">
              <span class="bar" :class="{ checking: r.state === 'checking' }" role="progressbar" :aria-valuenow="percentOf(r)" aria-valuemin="0" aria-valuemax="100"><i :style="{ width: `${percentOf(r)}%` }"></i></span>
              <span class="pc">{{ rowWord(r, run.op) }} <b>{{ percentOf(r) }}%</b></span>
            </span>
            <span v-else class="word" :class="{ failed: markOf(r.state) === 'failed' }">{{ rowWord(r, run.op) }}<template v-if="reasonOf(r)">: {{ reasonOf(r) }}</template></span>
          </li>
        </ul>
      </div>
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
.top { flex: none; display: flex; flex-direction: column; gap: 12px; padding: 4px 20px 12px; }
h1 { margin: 0; font-size: 28px; line-height: 1.08; font-weight: 800; letter-spacing: -0.5px; }
.lead { margin: -8px 0 0; }
.quiet { font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.top > p.quiet { margin: 0; }
.art { width: 200px; height: 100px; }
.art.small { width: 140px; height: 70px; }
.sum { display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap; }
.big { font-size: 40px; font-weight: 800; letter-spacing: -1px; line-height: 1; }
.blocks { display: grid; grid-template-columns: repeat(20, 1fr); gap: 3px; height: 26px; }
.blocks i { border: 2px solid var(--edge); border-radius: 3px; background: var(--panel); }
.blocks i.on { background: var(--tint-drain); }
/* The next block to fill blinks, in whole steps like the pixels. */
.blocks i.lead { animation: blink 1s steps(1) infinite; }
@keyframes blink { 50% { background: var(--tint-drain); } }
.counts { display: flex; flex-wrap: wrap; gap: 6px 16px; font-size: 15px; color: var(--text-quiet); }
.counts b { font-size: 17px; font-weight: 800; color: var(--text); }
.counts .failed b { color: var(--bad); }

/* The files: one list, every line one height (LINE). */
.files { flex: 1; min-height: 0; overflow-y: auto; margin: 0 20px; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.spacer { position: relative; }
.slice { margin: 0; padding: 0; list-style: none; will-change: transform; }
.file { box-sizing: border-box; height: 60px; display: grid; grid-template-columns: 24px minmax(0, 1fr) auto; grid-template-rows: auto auto; align-content: center; column-gap: 10px; row-gap: 4px; padding: 0 12px; border-top: 2px solid var(--rule); }
.file:first-child { border-top: 0; }
/* A file on its way is raised, as on the desktop's Transfer. */
.file.going { background: var(--surface-raised); }
.mark { grid-row: 1 / 3; align-self: center; width: 22px; height: 22px; display: grid; place-items: center; font-size: 13px; font-weight: 900; line-height: 1; border: 2px solid var(--edge); border-radius: 5px; }
.m-waiting { border-color: var(--text-faint); }
.m-sending { color: var(--accent-ink); background: var(--accent); animation: beat 0.9s steps(2) infinite; }
.m-checking { color: var(--warn-ink); background: var(--ok); animation: beat 0.9s steps(2) infinite; }
.m-done { color: var(--warn-ink); background: var(--ok); }
.m-same { color: var(--text); background: var(--surface-hover); }
.m-held { color: var(--warn-ink); background: var(--hold); }
.m-failed { color: var(--warn-ink); background: var(--bad); }
@keyframes beat { 50% { opacity: 0.55; } }
.name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 15px; font-weight: 700; }
.size { font-size: 13px; color: var(--text-faint); white-space: nowrap; }
.word { grid-column: 2 / -1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; color: var(--text-quiet); }
.word.failed { color: var(--bad); font-weight: 700; }
.meter { grid-column: 2 / -1; display: flex; align-items: center; gap: 10px; }
.bar { flex: 1; height: 10px; background: var(--surface); border: 2px solid var(--edge); border-radius: 3px; overflow: hidden; }
/* Stripes that travel along the filled part, as on the desktop: the file is
   moving even while its number has not changed. */
.bar i {
  display: block;
  height: 100%;
  background-color: var(--accent);
  background-image: repeating-linear-gradient(-45deg, transparent 0 6px, color-mix(in srgb, var(--accent-ink) 28%, transparent) 6px 12px);
  background-size: 17px 100%;
  animation: travel 0.9s linear infinite;
  transition: width 0.3s ease-out;
}
/* Checking reads it back from where it went: "it arrived" green, so it is
   not mistaken for a copy going backwards. */
.bar.checking i { background-color: var(--ok); }
@keyframes travel { to { background-position: 17px 0; } }
.pc { flex: none; font-size: 13px; color: var(--text-quiet); white-space: nowrap; }
.pc b { color: var(--text); }
@media (prefers-reduced-motion: reduce) {
  .bar i, .m-sending, .m-checking, .blocks i.lead { animation: none; }
}
.act { flex: none; padding: 10px 20px 12px; }
.pair { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
.col { display: flex; flex-direction: column; gap: 12px; }
</style>
