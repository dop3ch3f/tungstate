<!-- Moving files to another machine. Opens on the two panes, because picking
     files is the thing you came here to do; runs, saved pairs and connections
     are siblings one click away. -->
<script setup lang="ts">
import { computed, onMounted, ref, shallowRef, useTemplateRef, watch } from "vue";
import { transfers, links as linkApi } from "../../engine/commands";
import { useTransfer } from "../../state/useTransfer";
import { bytes } from "../../lib/format";
import type { Leg, Place, TransferRequest } from "../../engine/types";
import Pane from "./Pane.vue";
import RunView from "./RunView.vue";
import LinksView from "./LinksView.vue";
import { useConnections } from "../../state/useConnections";
import { useNav } from "../../nav";
import TransferSetup from "./TransferSetup.vue";
import Button from "../../ui/Button.vue";
import ActionBar from "../../ui/ActionBar.vue";
import Tile from "../../ui/Tile.vue";
import Notice from "../../ui/Notice.vue";
import Sheet from "../../ui/Sheet.vue";

type Where = "files" | "runs" | "links";
const t = useTransfer();
const where = ref<Where>(t.openOn.value ?? "files");
t.openOn.value = null;
const nav = useNav();
const cx = useConnections();

const places = shallowRef<Place[]>([]);
const leftStart = ref("/");
const rightStart = ref("/");
// The panes wait for the remembered folders. Opening them at "/" meanwhile got
// "/" remembered in their place whenever a remembered folder failed to open.
const ready = ref(false);
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
/** Where ticked files would go, by folder name: the panes already show the
 *  full paths, and two of them cut short wrapped across the footer. */
const route = computed(() => {
  if (!legs.value.length) return "";
  if (legs.value.length === 2) return `${leafOf(leftPath.value)} ⇄ ${leafOf(rightPath.value)}`;
  return `→ ${leafOf(legs.value[0]!.destination)}`;
});
const leafOf = (path: string) => path.replace(/[/:]+$/, "").split(/[/:]/).pop() || path;


// Which folders the panes were showing, kept for the next launch. Debounced,
// so walking through folders quickly does not write the file on every step.
let saveTimer: number | undefined;
watch([leftPath, rightPath], ([l, r]) => {
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    transfers.rememberPanes({ left: l || null, right: r || null }).catch(() => {});
  }, 400);
});

onMounted(async () => {
  try {
    places.value = await transfers.places();
    const home = places.value.find((p) => p.label === "Home")?.path ?? "/";
    const last = await transfers.lastPanes();
    leftStart.value = opening ?? last.left ?? places.value.find((p) => p.label === "Movies")?.path ?? home;
    rightStart.value = last.right ?? places.value.find((p) => p.path.startsWith("/Volumes"))?.path ?? home;
    ready.value = true;
    await t.refreshStranded();
  } catch (e) {
    problem.value = String(e);
  } finally {
    ready.value = true;
  }
});

/** Start, or join the queue. A queued transfer leaves you where you were,
 *  picking the next files, and says where it is in line. */
async function send(request: TransferRequest) {
  setup.value = null;
  try {
    const accepted = await transfers.start(request);
    t.accepted(accepted);
    if (accepted.started) where.value = "runs";
    await linkApi.list();
  } catch (e) {
    problem.value = String(e);
  }
  left.value?.reload();
  right.value?.reload();
}

const WHERE = { files: "Files", runs: "Runs", links: "Saved pairs" } as const;

// Browse, pressed in the Connections section: the left pane opens there,
// ahead of the folder it was last showing. Taken once, so coming back to
// Transfer later starts as usual.
const opening = cx.browseTo.value;
cx.browseTo.value = null;
</script>

<template>
  <div class="dh">
    <div class="head"><Tile of="drain" :size="26" /><h1 class="dh-title">Transfer</h1></div>
    <div class="dh-top">
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
    <Button look="link" @click="nav.go('connections')">Connections</Button>
    </div>

    <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
    <Notice v-if="t.placed.value && where !== 'runs'">
      {{ t.placed.value.line }}
      <Button look="link" @click="where = 'runs'">See the queue</Button>
    </Notice>
    <Notice tone="hold" v-for="run in t.stranded.value" :key="run.link">
      {{ run.link }} stopped part-way.
      <Button look="link" :busy="t.settling(run.link)" @click="t.resume(run.link); where = 'runs'">Resume</Button>
      or
      <Button look="link" :busy="t.settling(run.link)" @click="t.discard(run.link)">discard the part-copied files</Button>
    </Notice>

    <div class="dh-body" v-if="where === 'files'">
      <div class="dh-panes" v-if="ready">
        <Pane
          ref="left" :start="leftStart" :active="active === 'left'"
          @focus="active = 'left'" @located="leftPath = $event"
          @selection="(names, total) => { leftPicked = names; leftWeight = total }"
        />
        <Pane
          ref="right" :start="rightStart" :active="active === 'right'"
          @focus="active = 'right'" @located="rightPath = $event"
          @selection="(names, total) => { rightPicked = names; rightWeight = total }"
        />
      </div>
      <ActionBar>
        <template #say>
          <span v-if="picked">{{ picked }} ticked, {{ bytes(weight) }}</span>
          <span class="dh-dim" v-else>Tick files on either side.</span>
          <span class="dh-route">{{ route }}</span>
        </template>
        <Button :disabled="!picked" @click="setup = { legs, intent: 'copy' }">Copy</Button>
        <Button look="primary" :disabled="!picked" @click="setup = { legs, intent: 'move' }">Move</Button>
      </ActionBar>
    </div>

    <div class="dh-body dh-pad" v-else-if="where === 'runs'"><RunView /></div>
    <div class="dh-body dh-pad" v-else><LinksView :places="places" @ran="where = 'runs'" /></div>

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
    <Sheet v-if="t.conflict.value" of="drain" :title="`A different ${leafOf(t.conflict.value.path)} is already there`" @dismiss="t.answerConflict('skip', false); applyAll = false">
      <p class="dh-qw">
        This one is {{ bytes(t.conflict.value.incoming_size) }}; the one there is
        {{ bytes(t.conflict.value.existing_size) }}. Setting it aside leaves the one
        there alone and puts this one in the set-aside folder beside it, to look at later.
      </p>
      <label class="dh-tick"><input type="checkbox" v-model="applyAll" /> Do the same for the rest</label>
      <div class="dh-qf">
        <Button @click="t.answerConflict('skip', applyAll); applyAll = false">Skip it</Button>
        <Button @click="t.answerConflict('rename', applyAll); applyAll = false">Keep both</Button>
        <Button look="primary" @click="t.answerConflict('quarantine', applyAll); applyAll = false">Set this one aside</Button>
      </div>
    </Sheet>

    <Sheet v-if="t.identical.value" of="drain" :title="`${leafOf(t.identical.value.path)} is already there`" @dismiss="t.answerIdentical(false, false); applyAll = false">
      <p class="dh-qw">
        The copy there is the same, byte for byte, so nothing needs sending.
        Delete this one to free {{ bytes(t.identical.value.size) }}?
      </p>
      <label class="dh-tick"><input type="checkbox" v-model="applyAll" /> Do the same for the rest</label>
      <div class="dh-qf">
        <Button @click="t.answerIdentical(false, applyAll); applyAll = false">Keep it here</Button>
        <Button look="primary" @click="t.answerIdentical(true, applyAll); applyAll = false">Delete it here</Button>
      </div>
    </Sheet>
  </div>
</template>

<style scoped>
.head { display: flex; align-items: center; gap: var(--s3); }
.dh { position: absolute; inset: 0; display: flex; flex-direction: column; padding: var(--win-pad); padding-bottom: var(--s3); gap: var(--s3); }
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

/* Retro: a tab is a block with an outline, and the one you are in is white and bold, like the section in the rail. */
/* Retro: one joined strip in an outline, no shadow, so a tab never reads
   as a button; the one you are in is filled and bold, like the rail. */
:global([data-theme="retro"] .dh-tabs) {
  background: var(--surface-raised);
  padding: 0;
  gap: 0;
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius);
  overflow: hidden;
}
:global([data-theme="retro"] .dh-tabs button) {
  border: none;
  border-right: var(--bw) solid var(--edge);
  border-radius: 0;
  background: none;
  color: var(--text);
  padding: 5px var(--s3);
}
:global([data-theme="retro"] .dh-tabs button:last-child) { border-right: none; }
:global([data-theme="retro"] .dh-tabs .dh-on) {
  background: var(--chosen);
  color: var(--chosen-ink);
  font-weight: 700;
}
.dh-badge { color: var(--accent); font-size: 9px; vertical-align: 2px; }
.dh-top { display: flex; align-items: center; justify-content: space-between; gap: var(--s3); }

.dh-body { flex: 1; min-height: 0; display: flex; flex-direction: column; gap: var(--s3); }
.dh-pad { padding-right: var(--s1); }
.dh-pad { overflow-y: auto; }
.dh-panes { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: var(--s3); }

.dh-dim { color: var(--text-faint); }
.dh-route { color: var(--text-faint); word-break: normal; }
.dh-qw { font-size: var(--small); color: var(--text-quiet); margin: var(--s2) 0 0; line-height: 1.5; }
.dh-tick { display: flex; gap: var(--s2); font-size: var(--small); color: var(--text-quiet); margin-top: var(--s3); }
.dh-qf { display: flex; justify-content: flex-end; gap: var(--s2); margin-top: var(--s5); flex-wrap: wrap; }
</style>
