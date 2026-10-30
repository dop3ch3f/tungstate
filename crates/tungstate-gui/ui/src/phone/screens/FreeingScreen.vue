<!-- A Free up run, over the whole screen with no tabs: sending, paused, how
     it ended, and clearing what was checked. Matches the approved running
     and done mockups. What clearing means differs by platform: Android
     deletes for good once the person agrees; iPhone moves photos to Recently
     Deleted, which only the person can empty. -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { bytes, duration, plural } from "../../lib/format";
import { isAndroid } from "../platform";
import { useFreeUp } from "../state/useFreeUp";
import { usePhotos } from "../state/usePhotos";
import PScreen from "../ui/PScreen.vue";
import PButton from "../ui/PButton.vue";
import PScene from "../ui/PScene.vue";

const run = useFreeUp();
const lib = usePhotos();

// The time left is worked out from the clock, so it is re-read now and then
// even between ticks; a stalled run should not keep promising a minute.
const now = ref(Date.now());
let timer: ReturnType<typeof setInterval> | undefined;
onMounted(() => (timer = setInterval(() => (now.value = Date.now()), 1000)));
onUnmounted(() => clearInterval(timer));

const where = computed(() => (run.asked.value ? `${run.asked.value.connection} › ${run.asked.value.folder}` : ""));
const sending = computed(() => ["starting", "running", "pausing", "stopping"].includes(run.phase.value));

const BLOCKS = 20;
const filled = computed(() => {
  const t = run.tick.value;
  return t && t.total ? Math.round((t.bytes / t.total) * BLOCKS) : 0;
});

const sub = computed(() => {
  const t = run.tick.value;
  if (!t) return "Getting ready…";
  const left = run.phase.value === "running" ? run.secondsLeft(now.value) : null;
  return left === null ? `of ${bytes(t.total)}` : `of ${bytes(t.total)} · about ${duration(left)} left`;
});

const title = computed(() => {
  const r = run.ran.value;
  if (run.phase.value === "paused") return "Paused";
  if (run.phase.value !== "done") return "Freeing space";
  if (r?.checked) return `${plural(r.checked, "photo or video is", "photos and videos are")} safe`;
  return r?.stopped && !r.problem ? "Stopped" : "Nothing was sent";
});

const how = computed(() =>
  run.asked.value?.verify === "size"
    ? "Each one's size was checked there before it could leave this phone."
    : "Each was read back and matched before it could leave this phone.",
);

const android = isAndroid();
const checked = computed(() => run.ran.value?.checked ?? 0);
/** Every checked one went, so the bytes freed are the bytes checked. */
const allCleared = computed(() => !!run.cleared.value && run.cleared.value.removed === checked.value);
const declined = computed(() => run.cleared.value?.supported === true && run.cleared.value.removed === 0);

function done() {
  run.finish();
  void lib.load();
}
</script>

<template>
  <PScreen :title="title">
    <div class="stack">
      <template v-if="run.phase.value !== 'done'">
        <PScene of="send" class="art" />
        <div class="centre">
          <div class="big">{{ bytes(run.tick.value?.bytes ?? 0) }}</div>
          <div class="quiet">{{ sub }}</div>
        </div>
        <div class="blocks" aria-hidden="true">
          <i v-for="n in BLOCKS" :key="n" :class="{ on: n <= filled }"></i>
        </div>
        <div v-if="run.tick.value" class="quiet centre">
          {{ run.tick.value.checked.toLocaleString() }} of {{ run.tick.value.count.toLocaleString() }} checked on {{ run.asked.value?.connection }}
        </div>
        <div v-if="run.phase.value === 'paused'" class="note">
          Nothing more is sent until you resume. What was sent stays on {{ run.asked.value?.connection }}, and is not sent again.
        </div>
        <div v-else-if="run.phase.value === 'pausing' || run.phase.value === 'stopping'" class="note">
          Finishing {{ run.tick.value?.current || "the one in hand" }} first, so nothing is left half sent.
        </div>
        <div v-else class="note">
          Keep Tungstate open until this finishes; the screen stays on meanwhile. Locking the phone or switching apps can pause sending.
        </div>
      </template>

      <template v-else>
        <p class="quiet lead">
          <template v-if="run.ran.value?.checked">On {{ where }}. {{ how }}</template>
          <template v-else-if="run.ran.value?.problem">{{ run.ran.value.problem }}</template>
          <template v-else-if="run.ran.value?.stopped">Nothing was sent, and nothing left this phone.</template>
          <template v-else>None of them could be sent, so all {{ (run.ran.value?.kept ?? 0).toLocaleString() }} stay on this phone.</template>
        </p>
        <PScene v-if="checked" of="done" class="art" />
        <div v-if="run.clearing.value" class="box">
          <div class="quiet">Clearing from this phone</div>
          <div class="wait">Waiting for you to allow it…</div>
        </div>
        <div v-else-if="run.cleared.value?.removed" class="box">
          <div class="quiet">Freed on this phone</div>
          <div class="big small">{{ allCleared ? bytes(run.ran.value!.checked_bytes) : plural(run.cleared.value.removed, "item") }}</div>
        </div>
        <div v-else-if="checked" class="box">
          <div class="quiet">Safe on {{ run.asked.value?.connection }}</div>
          <div class="big small">{{ bytes(run.ran.value!.checked_bytes) }}</div>
        </div>
        <div v-if="run.cleared.value?.removed" class="note plain">
          <template v-if="android">Android has deleted them from this phone for good. The copies on {{ where }} are now the only ones.</template>
          <template v-else>Photos keeps what was cleared for 30 days. To get the space back today, open Photos › Recently Deleted and choose Delete All.</template>
        </div>
        <div v-else-if="declined" class="note plain">They are still on this phone as well, because removing them was not allowed.</div>
        <div v-else-if="run.cleared.value && !run.cleared.value.supported" class="note plain">
          This version of Android cannot let apps remove photos. Delete them in your gallery; each one is on {{ where }}.
        </div>
        <p v-if="run.ran.value?.kept && !run.ran.value.checked && run.ran.value.problem" class="quiet">
          {{ run.ran.value.kept === 1 ? "It stays" : `All ${run.ran.value.kept.toLocaleString()} stay` }} on this phone.
        </p>
        <p v-else-if="run.ran.value?.kept && run.ran.value.checked" class="quiet">
          {{ plural(run.ran.value.kept, "other stays", "others stay") }} on this phone: {{ run.ran.value.kept === 1 ? "it was" : "they were" }} not checked on {{ run.asked.value?.connection }}{{ run.ran.value.stopped ? " before you stopped" : "" }}.
        </p>
        <p v-if="run.ran.value?.reason && run.ran.value.kept" class="quiet">{{ run.ran.value.kept === 1 ? "Why" : "Why the first stayed" }}: {{ run.ran.value.reason }}.</p>
        <p v-if="run.ran.value?.checked && run.ran.value?.problem" class="bad">{{ run.ran.value.problem }}</p>
        <p v-if="run.problem.value" class="bad">{{ run.problem.value }}</p>
      </template>
    </div>

    <template #act>
      <div v-if="sending" class="pair">
        <PButton quiet :busy="run.phase.value === 'pausing'" :disabled="run.phase.value !== 'running'" @press="run.pause()">
          {{ run.phase.value === "pausing" ? "Pausing…" : "Pause" }}
        </PButton>
        <PButton quiet :busy="run.phase.value === 'stopping'" :disabled="run.phase.value !== 'running'" @press="run.stop()">
          {{ run.phase.value === "stopping" ? "Stopping…" : "Stop" }}
        </PButton>
      </div>
      <div v-else-if="run.phase.value === 'paused'" class="col">
        <PButton @press="run.resume()">Resume</PButton>
        <PButton quiet @press="run.stop()">Stop</PButton>
      </div>
      <div v-else-if="declined" class="col">
        <PButton :busy="run.clearing.value" @press="run.clear()">Clear {{ plural(checked, "item") }} from this phone</PButton>
        <PButton quiet @press="done">Done</PButton>
      </div>
      <PButton v-else :busy="run.clearing.value" @press="done">Done</PButton>
    </template>
  </PScreen>
</template>

<style scoped>
.stack { display: flex; flex-direction: column; gap: 20px; }
.art { margin-top: 10px; }
.centre { text-align: center; }
.big { font-size: 64px; font-weight: 800; letter-spacing: -1.5px; line-height: 0.95; }
.small { font-size: 44px; margin-top: 6px; }
.quiet { font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.centre .quiet { margin-top: 8px; }
.lead { margin: -8px 0 0; }
.blocks { display: grid; grid-template-columns: repeat(20, 1fr); gap: 3px; height: 34px; }
.blocks i { border: 2px solid var(--edge); border-radius: 3px; background: var(--panel); }
.blocks i.on { background: var(--tint-drain); }
.note { padding: 14px 16px; font-size: 16px; line-height: 1.4; color: var(--warn-ink); background: var(--warn-bar); border: 2px solid var(--edge); border-radius: 8px; }
.wait { margin-top: 8px; font-size: 20px; font-weight: 800; }
.plain { color: var(--text); background: var(--panel); }
.box { padding: 16px; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.bad { margin: 0; font-size: 15px; font-weight: 700; color: var(--bad); }
.pair { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
.col { display: flex; flex-direction: column; gap: 12px; }
</style>
