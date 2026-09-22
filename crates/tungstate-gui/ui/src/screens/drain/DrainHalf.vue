<!-- Moving files to another machine. Opens on the two panes, because picking
     files is the thing you came here to do; runs, saved pairs and connections
     are siblings one click away. -->
<script setup lang="ts">
import { computed, onMounted, ref, shallowRef, useTemplateRef } from "vue";
import { transfers, links as linkApi } from "../../engine/commands";
import { useTransfer } from "../../state/useTransfer";
import { bytes, shortPath } from "../../lib/format";
import type { Leg, Place, TransferRequest } from "../../engine/types";
import Pane from "./Pane.vue";
import RunView from "./RunView.vue";
import LinksView from "./LinksView.vue";
import ConnectionsView from "./ConnectionsView.vue";
import TransferSetup from "./TransferSetup.vue";
import Button from "../../ui/Button.vue";
import Tile from "../../ui/Tile.vue";
import Notice from "../../ui/Notice.vue";
import Sheet from "../../ui/Sheet.vue";

type Where = "files" | "runs" | "links" | "connections";
const where = ref<Where>("files");
const t = useTransfer();

const places = shallowRef<Place[]>([]);
const leftStart = ref("/");
const rightStart = ref("/");
const problem = ref<string | null>(null);

const left = useTemplateRef<InstanceType<typeof Pane>>("left");
const right = useTemplateRef<InstanceType<typeof Pane>>("right");
const leftPath = ref("");
const rightPath = ref("");
const leftPicked = ref<string[]>([]);
const rightPicked = ref<string[]>([]);
const leftWeight = ref(0);
const rightWeight = ref(0);
const active = ref<"left" | "right">("left");

const setup = ref<{ legs: Leg[]; intent: "move" | "copy" } | null>(null);
const applyAll = ref(false);

/** Which way the files go is decided by which side has ticks, not by which
 *  pane was clicked last. Both ticked is an exchange, and says so. */
const legs = computed<Leg[]>(() => {
  const out: Leg[] = [];
  if (leftPicked.value.length) out.push({ source: leftPath.value, destination: rightPath.value, names: leftPicked.value });
  if (rightPicked.value.length) out.push({ source: rightPath.value, destination: leftPath.value, names: rightPicked.value });
  return out;
});
const picked = computed(() => leftPicked.value.length + rightPicked.value.length);
const weight = computed(() => leftWeight.value + rightWeight.value);
const route = computed(() => {
  if (!legs.value.length) return "";
  if (legs.value.length === 2) return `${shortPath(leftPath.value)} ⇄ ${shortPath(rightPath.value)}`;
  const leg = legs.value[0];
  return `${shortPath(leg.source)} → ${shortPath(leg.destination)}`;
});

onMounted(async () => {
  try {
    places.value = await transfers.places();
    const home = places.value.find((p) => p.label === "Home")?.path ?? "/";
    const last = await transfers.lastPanes();
    leftStart.value = last.left ?? places.value.find((p) => p.label === "Movies")?.path ?? home;
    rightStart.value = last.right ?? places.value.find((p) => p.path.startsWith("/Volumes"))?.path ?? home;
    await t.refreshStranded();
  } catch (e) {
    problem.value = String(e);
  }
});

async function send(request: TransferRequest) {
  setup.value = null;
  t.clearRun();
  where.value = "runs";
  try {
    await transfers.start(request);
    await linkApi.list();
  } catch (e) {
    problem.value = String(e);
  }
  left.value?.reload();
  right.value?.reload();
}

const WHERE = { files: "Files", runs: "Runs", links: "Saved pairs", connections: "Connections" } as const;
</script>

<template>
  <div class="dh">
    <div class="head"><Tile of="drain" :size="26" /><h1 class="dh-title">Move to another machine</h1></div>
    <nav class="dh-tabs">
      <button
        v-for="(label, key) in WHERE"
        :key="key"
        :class="{ 'dh-on': where === key }"
        @click="where = key"
      >
        {{ label }}
        <span class="dh-badge" v-if="key === 'runs' && t.running.value">●</span>
      </button>
    </nav>

    <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
    <Notice tone="hold" v-if="t.stranded.value.length" class="dh-unfinished">
      {{ t.stranded.value.length === 1 ? "A transfer" : `${t.stranded.value.length} transfers` }}
      stopped part-way.
      <template v-for="run in t.stranded.value" :key="run.link">
        <Button look="link" @click="t.resume(run.link); where = 'runs'">Pick up {{ run.link }}</Button>
        <Button look="link" @click="t.discard(run.link)">Clean it up</Button>
      </template>
    </Notice>

    <div class="dh-body" v-if="where === 'files'">
      <div class="dh-panes">
        <Pane
          ref="left" :start="leftStart" :places="places" :active="active === 'left'"
          @focus="active = 'left'" @located="leftPath = $event"
          @selection="(names, total) => { leftPicked = names; leftWeight = total }"
        />
        <Pane
          ref="right" :start="rightStart" :places="places" :active="active === 'right'"
          @focus="active = 'right'" @located="rightPath = $event"
          @selection="(names, total) => { rightPicked = names; rightWeight = total }"
        />
      </div>
      <footer class="dh-act">
        <span class="dh-tally" v-if="picked">{{ picked }} ticked, {{ bytes(weight) }}</span>
        <span class="dh-tally dh-dim" v-else>Tick files on either side.</span>
        <span class="path dh-route">{{ route }}</span>
        <Button :disabled="!picked" @click="setup = { legs, intent: 'copy' }">Copy</Button>
        <Button look="primary" :disabled="!picked" @click="setup = { legs, intent: 'move' }">Move</Button>
      </footer>
    </div>

    <div class="dh-body dh-pad" v-else-if="where === 'runs'"><RunView /></div>
    <div class="dh-body dh-pad" v-else-if="where === 'links'"><LinksView @ran="where = 'runs'" /></div>
    <div class="dh-body dh-pad" v-else><ConnectionsView /></div>

    <TransferSetup
      v-if="setup"
      :legs="setup.legs"
      :intent="setup.intent"
      @dismiss="setup = null"
      @go="send"
    />

    <!-- The two questions a run can stop and ask. `applyAll` is reset after
         every answer: a shared tick that persisted would silently apply the
         first decision to every later file. -->
    <Sheet v-if="t.conflict.value" @dismiss="t.answerConflict('skip', false); applyAll = false">
      <h2 class="dh-qt">A different file of that name is already there</h2>
      <p class="path">{{ t.conflict.value.path }}</p>
      <p class="dh-qw">
        Coming: {{ bytes(t.conflict.value.incoming_size) }}. Already there:
        {{ bytes(t.conflict.value.existing_size) }}.
      </p>
      <label class="dh-tick"><input type="checkbox" v-model="applyAll" /> do this for the rest of them</label>
      <div class="dh-qf">
        <Button @click="t.answerConflict('skip', applyAll); applyAll = false">Leave it here</Button>
        <Button @click="t.answerConflict('rename', applyAll); applyAll = false">Keep both</Button>
        <Button look="primary" @click="t.answerConflict('quarantine', applyAll); applyAll = false">Set the other one aside</Button>
      </div>
    </Sheet>

    <Sheet v-if="t.identical.value" @dismiss="t.answerIdentical(false, false); applyAll = false">
      <h2 class="dh-qt">That file is already there, byte for byte</h2>
      <p class="path">{{ t.identical.value.path }}</p>
      <p class="dh-qw">
        Nothing needs sending. You asked to move, so the only question is
        whether to remove this copy and get back {{ bytes(t.identical.value.size) }}.
      </p>
      <label class="dh-tick"><input type="checkbox" v-model="applyAll" /> do this for the rest of them</label>
      <div class="dh-qf">
        <Button @click="t.answerIdentical(false, applyAll); applyAll = false">Keep it</Button>
        <Button look="primary" @click="t.answerIdentical(true, applyAll); applyAll = false">Remove this copy</Button>
      </div>
    </Sheet>
  </div>
</template>

<style scoped>
.head { display: flex; align-items: center; gap: var(--s3); }
.dh { position: absolute; inset: 0; display: flex; flex-direction: column; padding: var(--s5) var(--s5) var(--s3); gap: var(--s3); }
.dh-title { font-size: var(--display); font-weight: 700; letter-spacing: -0.01em; margin: 0; }

.dh-tabs { display: flex; gap: 2px; padding: 3px; background: var(--rail); border-radius: var(--radius); align-self: flex-start; }
.dh-tabs button {
  font: inherit;
  font-size: var(--small);
  background: none;
  border: none;
  color: var(--text-faint);
  padding: var(--s1) var(--s3);
  border-radius: var(--radius);
  cursor: pointer;
}
.dh-tabs button:hover { color: var(--text-quiet); }
.dh-tabs .dh-on { color: var(--text); background: var(--surface-raised); }
.dh-badge { color: var(--accent); font-size: 9px; vertical-align: 2px; }
.dh-unfinished { display: flex; flex-wrap: wrap; gap: var(--s3); align-items: baseline; }

.dh-body { flex: 1; min-height: 0; display: flex; flex-direction: column; gap: var(--s3); }
.dh-pad { overflow-y: auto; }
.dh-panes { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: var(--s3); }

.dh-act { display: flex; align-items: center; gap: var(--s3); }
.dh-tally { font-size: var(--small); color: var(--text-quiet); }
.dh-dim { color: var(--text-faint); }
.dh-route { color: var(--text-faint); margin-right: auto; word-break: normal; }

.dh-qt { font-size: var(--body); font-weight: 600; margin: 0 0 var(--s2); }
.dh-qw { font-size: var(--small); color: var(--text-quiet); margin: var(--s2) 0 0; line-height: 1.5; }
.dh-tick { display: flex; gap: var(--s2); font-size: var(--small); color: var(--text-quiet); margin-top: var(--s3); }
.dh-qf { display: flex; justify-content: flex-end; gap: var(--s2); margin-top: var(--s5); flex-wrap: wrap; }
</style>
