<!-- Free up space: what the photo library could give back, chosen by kind,
     age and year, where it goes, and how carefully each one is checked
     there. Free starts the run, which takes over the screen. Matches the
     approved mockup. -->
<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import type { CheckLevel, PhotoFilter } from "../../engine/types";
import { bytes, plural } from "../../lib/format";
import { useConnections } from "../../state/useConnections";
import { usePhoneNav } from "../nav";
import { AGES, KINDS, usePhotos } from "../state/usePhotos";
import { useFreeUp } from "../state/useFreeUp";
import PScreen from "../ui/PScreen.vue";
import PRow from "../ui/PRow.vue";
import PTile from "../ui/PTile.vue";
import PButton from "../ui/PButton.vue";

const lib = usePhotos();
const run = useFreeUp();
const conns = useConnections();
const nav = usePhoneNav();
onMounted(() => {
  void lib.load();
  void conns.load();
});

const sheet = ref<"filter" | "target" | null>(null);
const targets = computed(() => conns.all.value.filter((c) => c.scheme !== "fs"));
const chosenTarget = ref<string | null>(null);
const target = computed(() => targets.value.find((c) => c.name === chosenTarget.value) ?? targets.value[0] ?? null);

const filterLine = computed(() => {
  const kind = KINDS.find((k) => k.id === lib.filter.value.kind)!.label;
  const age = AGES.find((a) => a.days === lib.filter.value.olderThanDays)!;
  return age.days ? `${kind} · older than ${age.label}` : `${kind} · any age`;
});
const pick = (next: Partial<PhotoFilter>) => void lib.setFilter({ ...lib.filter.value, ...next });

/** Where under the connection photos go: `Phone/<year>/<kind>`. */
const FOLDER = "Phone";

// Each level says what it costs, so the choice is an informed one.
const CHECKS: { id: CheckLevel; label: string; says: (to: string) => string }[] = [
  { id: "readback", label: "Read back", says: (to) => `Reads each copy back from ${to}. Slower, and the only check that proves the bytes there are right.` },
  { id: "size", label: "Size only", says: () => "Only compares sizes. Faster, but a copy damaged on the way would not be caught." },
];
const checkSays = computed(() => CHECKS.find((c) => c.id === run.check.value)!.says(target.value?.name ?? "the connection"));

function free() {
  if (!target.value) return;
  void run.start({
    connection: target.value.name,
    folder: FOLDER,
    filter: lib.filter.value,
    years_left_out: [...lib.left.value],
    favourites: lib.favourites.value,
    verify: run.check.value,
  });
}
</script>

<template>
  <PScreen title="Free up space">
    <!-- Before the person has said the app may look. -->
    <div v-if="lib.access.value === 'ask'" class="card">
      <PTile art="drain" tint="var(--tint-drain)" light :size="48" />
      <b>Let Tungstate see your photos</b>
      <p>To count what can be freed. Nothing leaves this phone until you choose to send it.</p>
    </div>
    <div v-else-if="lib.access.value === 'denied'" class="card">
      <PTile art="drain" tint="var(--tint-drain)" light :size="48" />
      <b>Photo access is turned off</b>
      <p>Turn it on in Settings › Apps › Tungstate › Permissions › Photos and videos, then come back.</p>
    </div>

    <template v-else-if="lib.canRead.value">
      <p v-if="lib.access.value === 'limited'" class="line">Tungstate sees only the photos you picked.</p>
      <div class="box">
        <div class="hero">
          <span class="big">{{ bytes(lib.chosen.value.bytes) }}</span>
          <span class="quiet">{{ plural(lib.chosen.value.count, "item") }}</span>
        </div>
        <button class="choice press" @click="sheet = 'filter'">
          <span class="grow">{{ filterLine }}</span><span class="chev" aria-hidden="true">▾</span>
        </button>
        <button v-if="target" class="choice press" @click="sheet = 'target'">
          <PTile art="connections" tint="var(--tint-connections)" light :size="30" />
          <span class="grow">To {{ target.name }} › {{ FOLDER }}</span><span class="chev" aria-hidden="true">›</span>
        </button>
        <button v-else class="choice press" @click="nav.go('connections')">
          <span class="grow">Add a connection to send to</span><span class="chev" aria-hidden="true">›</span>
        </button>
      </div>

      <h2>Include</h2>
      <p v-if="lib.loading.value" class="line">Counting…</p>
      <p v-else-if="!lib.summary.value?.years.length && !lib.summary.value?.favourites.count" class="line">Nothing here matches.</p>
      <div v-else class="list">
        <button v-for="y in lib.summary.value?.years ?? []" :key="y.year" class="pick" @click="lib.toggleYear(y.year)">
          <span class="check" :class="{ on: !lib.left.value.has(y.year) }" aria-hidden="true">✓</span>
          <span class="text"><b>{{ y.year }}</b><span class="quiet">{{ plural(y.count, "item") }}</span></span>
          <span class="num">{{ bytes(y.bytes) }}</span>
        </button>
        <button v-if="lib.summary.value?.favourites.count" class="pick" @click="lib.favourites.value = !lib.favourites.value">
          <span class="check" :class="{ on: lib.favourites.value }" aria-hidden="true">✓</span>
          <span class="text"><b>Favourites</b><span class="quiet">{{ lib.favourites.value ? "Sent too" : "Stay on the phone" }}</span></span>
          <span class="num">{{ bytes(lib.summary.value.favourites.bytes) }}</span>
        </button>
      </div>
    </template>

    <p v-if="lib.problem.value" class="line bad">{{ lib.problem.value }}</p>
    <p v-if="run.problem.value" class="line bad">{{ run.problem.value }}</p>

    <!-- Bottom sheets: what to send, and where. -->
    <div v-if="sheet" class="veil" @click="sheet = null"></div>
    <div v-if="sheet === 'filter'" class="sheet">
      <div class="grab"></div>
      <h3>What to send</h3>
      <h2>Kind</h2>
      <div class="seg">
        <button v-for="k in KINDS" :key="k.id" :class="{ on: lib.filter.value.kind === k.id }" @click="pick({ kind: k.id })">{{ k.label }}</button>
      </div>
      <h2>Older than</h2>
      <div class="seg">
        <button v-for="a in AGES" :key="a.days" :class="{ on: lib.filter.value.olderThanDays === a.days }" @click="pick({ olderThanDays: a.days })">{{ a.label }}</button>
      </div>
      <h2>Check each one</h2>
      <div class="seg">
        <button v-for="c in CHECKS" :key="c.id" :class="{ on: run.check.value === c.id }" @click="run.setCheck(c.id)">{{ c.label }}</button>
      </div>
      <p class="says">{{ checkSays }}</p>
      <div class="sheet-act"><PButton @press="sheet = null">{{ lib.loading.value ? "Counting…" : `Show ${bytes(lib.chosen.value.bytes)}` }}</PButton></div>
    </div>
    <div v-if="sheet === 'target'" class="sheet">
      <div class="grab"></div>
      <h3>Send to</h3>
      <div class="list">
        <PRow v-for="c in targets" :key="c.name" :title="c.name" :sub="c.place" chevron @press="chosenTarget = c.name; sheet = null">
          <template #lead><PTile art="connections" tint="var(--tint-connections)" light /></template>
        </PRow>
      </div>
    </div>

    <template #act>
      <template v-if="lib.access.value === 'ask'">
        <PButton @press="lib.ask()">Allow access</PButton>
      </template>
      <template v-else-if="lib.canRead.value">
        <p v-if="target" class="say">Each item is checked on {{ target.name }} before it leaves</p>
        <PButton :disabled="!target || !lib.chosen.value.count || lib.loading.value" :busy="run.phase.value === 'starting'" @press="free">Free {{ bytes(lib.chosen.value.bytes) }}</PButton>
      </template>
    </template>
  </PScreen>
</template>

<style scoped>
.card { display: flex; flex-direction: column; gap: 12px; padding: 18px; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.card b { font-size: 19px; }
.card p { margin: 0; font-size: 16px; line-height: 1.4; color: var(--text-quiet); }
.box { display: flex; flex-direction: column; gap: 12px; padding: 16px; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.hero { display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap; }
.big { font-size: 52px; font-weight: 800; letter-spacing: -1.5px; line-height: 0.95; }
.quiet { font-size: 15px; color: var(--text-quiet); }
.choice {
  display: flex; align-items: center; gap: 12px; width: 100%; min-height: 52px; padding: 0 16px;
  font: inherit; font-size: 16px; font-weight: 700; text-align: left; color: var(--text);
  background: var(--panel); border: 2px solid var(--edge); border-radius: 8px;
}
.press { box-shadow: 3px 3px 0 var(--edge); }
.press:active { transform: translate(3px, 3px); box-shadow: none; }
.grow { flex: 1; }
.chev { font-size: 20px; font-weight: 800; }
h2 { font-size: 15px; font-weight: 800; margin: 22px 0 8px; }
h3 { font-size: 24px; font-weight: 800; margin: 0 0 4px; }
.line { margin: 4px 2px 12px; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.bad { color: var(--bad); font-weight: 700; }
.list { background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; overflow: hidden; }
.pick {
  display: flex; align-items: center; gap: 12px; width: 100%; min-height: 64px; padding: 12px 14px;
  font: inherit; text-align: left; color: var(--text); background: var(--panel);
  border: 0; border-top: 2px solid var(--edge);
}
.pick:first-child { border-top: 0; }
.check { width: 30px; height: 30px; flex: none; display: grid; place-items: center; font-weight: 900; color: transparent; background: var(--panel); border: 2px solid var(--edge); border-radius: 6px; }
.check.on { color: #ffffff; background: var(--tint-drain); }
.text { flex: 1; display: flex; flex-direction: column; gap: 2px; }
.text b { font-size: 16px; }
.num { font-size: 16px; font-weight: 800; }
.veil { position: fixed; inset: 0; background: var(--veil); z-index: 1; }
.sheet {
  position: fixed; left: 0; right: 0; bottom: 0; z-index: 2;
  /* Clear of the home bar on iPhone and the gesture bar on Android. */
  padding: 10px 20px calc(env(safe-area-inset-bottom, 0px) + 22px);
  background: var(--panel); border-top: 2px solid var(--edge); border-radius: 16px 16px 0 0;
}
.grab { width: 44px; height: 5px; margin: 0 auto 14px; border-radius: 3px; background: var(--edge); }
.seg { display: flex; overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.seg button { flex: 1; min-height: 48px; padding: 0 4px; font: inherit; font-size: 15px; font-weight: 700; color: var(--text); background: none; border: 0; border-right: 2px solid var(--edge); }
.seg button:last-child { border-right: 0; }
.seg button.on { color: var(--panel); background: var(--edge); }
.sheet-act { margin-top: 18px; }
.says { margin: 10px 2px 0; font-size: 14px; line-height: 1.4; color: var(--text-quiet); }
.say { margin: 0 2px 10px; font-size: 14px; text-align: center; color: var(--text-quiet); }
</style>
