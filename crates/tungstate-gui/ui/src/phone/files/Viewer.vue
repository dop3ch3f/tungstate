<!-- The image viewer: the photos of one folder, full screen, on the retro
     dark desk (docs: ~/.tungstate-demo/phone-mockups/es, "viewer"). Swipe
     sideways for the next one; pinch or double-tap to zoom. The picture is a
     screen-sized one made on the phone, so HEIC shows and a 50 MB photo does
     not have to load whole. -->
<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import type { Entry } from "../../engine/types";
import { filePicture } from "../../engine/pictures";
import { bytes, when } from "../../lib/format";
import Pixels from "../../ui/Pixels.vue";
import { useBack } from "../lib/back";
import PLoadBar from "../ui/PLoadBar.vue";

const props = defineProps<{ items: Entry[]; start: number }>();
const emit = defineEmits<{ close: []; share: [Entry]; move: [Entry]; info: [Entry]; delete: [Entry] }>();
useBack(() => (emit("close"), true));

const at = ref(Math.min(props.start, props.items.length - 1));

// The phone frame's own inset properties, inherited down to here; read off
// the element because the viewer covers the bars the frame pads for.
const box = ref<HTMLElement | null>(null);
const pad = ref<Record<string, string>>({});
onMounted(() => {
  if (!box.value) return;
  const style = getComputedStyle(box.value);
  const read = (name: string) => style.getPropertyValue(name).trim() || "0px";
  pad.value = { paddingTop: read("--top"), paddingBottom: read("--bottom") };
});
const here = computed(() => props.items[at.value]);
watch(() => props.items.length, (n) => {
  if (!n) emit("close");
  else if (at.value >= n) at.value = n - 1;
});

// The phone renders this many pixels across at most, so asking for more
// only costs time.
const EDGE = Math.min(1600, Math.round(Math.max(screen.width, screen.height) * (window.devicePixelRatio || 1)));
const src = (e: Entry | undefined) => (e ? filePicture(e.path, EDGE) : "");
const loaded = ref(false);
const failed = ref(false);
watch(at, () => {
  loaded.value = false;
  failed.value = false;
  zoom.value = 1;
  pan.value = { x: 0, y: 0 };
});

// ── gestures ────────────────────────────────────────────────────────────
// Two fingers zoom; one finger pans when zoomed and swipes when not.
const zoom = ref(1);
const pan = ref({ x: 0, y: 0 });
const pointers = new Map<number, { x: number; y: number }>();
let start: { x: number; y: number; zoom: number; span: number; pan: { x: number; y: number } } | null = null;
let lastTap = 0;

const span = () => {
  const [a, b] = [...pointers.values()];
  return a && b ? Math.hypot(a.x - b.x, a.y - b.y) : 0;
};
function down(e: PointerEvent) {
  (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
  start = { x: e.clientX, y: e.clientY, zoom: zoom.value, span: span(), pan: { ...pan.value } };
}
function move(e: PointerEvent) {
  if (!pointers.has(e.pointerId) || !start) return;
  pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
  if (pointers.size === 2 && start.span > 0) {
    zoom.value = Math.min(5, Math.max(1, start.zoom * (span() / start.span)));
  } else if (pointers.size === 1 && zoom.value > 1) {
    pan.value = { x: start.pan.x + (e.clientX - start.x), y: start.pan.y + (e.clientY - start.y) };
  }
}
function up(e: PointerEvent) {
  const begun = start;
  pointers.delete(e.pointerId);
  if (pointers.size) return;
  start = null;
  if (!begun) return;
  const dx = e.clientX - begun.x;
  const dy = e.clientY - begun.y;
  if (zoom.value <= 1.02 && Math.abs(dx) > 60 && Math.abs(dy) < 80) {
    step(dx < 0 ? 1 : -1);
    return;
  }
  if (Math.abs(dx) < 8 && Math.abs(dy) < 8) {
    const now = Date.now();
    if (now - lastTap < 300) {
      zoom.value = zoom.value > 1 ? 1 : 2.5;
      pan.value = { x: 0, y: 0 };
    }
    lastTap = now;
  }
  if (zoom.value <= 1.02) {
    zoom.value = 1;
    pan.value = { x: 0, y: 0 };
  }
}
function step(by: number) {
  const next = at.value + by;
  if (next >= 0 && next < props.items.length) at.value = next;
}
const transform = computed(() => `translate(${pan.value.x}px, ${pan.value.y}px) scale(${zoom.value})`);
</script>

<template>
  <div ref="box" class="viewer" role="dialog" :aria-label="here?.name" :style="pad">
    <div class="top">
      <button class="back" aria-label="Close" @click="emit('close')">‹</button>
      <div class="grow">
        <b>{{ here?.name }}</b>
        <span>{{ at + 1 }} of {{ items.length }}<template v-if="here?.size"> · {{ bytes(here.size) }}</template><template v-if="here?.modified"> · {{ when(here.modified) }}</template></span>
      </div>
    </div>
    <div class="stage" @pointerdown="down" @pointermove="move" @pointerup="up" @pointercancel="up">
      <p v-if="failed" class="trouble">This photo could not be shown here. Info › Open with… may open it in another app.</p>
      <div v-else-if="!loaded" class="wait"><PLoadBar label="Opening this photo" /></div>
      <img v-if="here && !failed" :key="here.path" :src="src(here)" :alt="here.name" :style="{ transform }" :class="{ on: loaded }" draggable="false" @load="loaded = true" @error="failed = true" />
      <!-- The next one, fetched while this one is looked at. -->
      <img v-if="items[at + 1]" class="ahead" :src="src(items[at + 1])" alt="" aria-hidden="true" />
    </div>
    <nav class="acts">
      <button @click="here && emit('share', here)"><Pixels of="share" :size="24" /><span>Share</span></button>
      <button @click="here && emit('move', here)"><Pixels of="move" :size="24" /><span>Move</span></button>
      <button @click="here && emit('info', here)"><Pixels of="info" :size="24" /><span>Info</span></button>
      <button class="bad" @click="here && emit('delete', here)"><Pixels of="trash" :size="24" /><span>Delete</span></button>
    </nav>
  </div>
</template>

<style scoped>
/* The retro dark desk with cream outlines, whatever the theme: a photo reads
   best on dark, and the cream keeps it tungstate's. */
.viewer {
  position: fixed; inset: 0; z-index: 4; display: flex; flex-direction: column;
  background: #23221e; color: #ece8d8;
}
.top { flex: none; display: flex; align-items: center; gap: 6px; padding: 6px 16px 10px; }
.back { width: 44px; height: 44px; flex: none; font: inherit; font-size: 30px; font-weight: 800; color: #ece8d8; background: none; border: 0; }
.grow { flex: 1; min-width: 0; }
.top b { display: block; font-size: 17px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.top span { font-size: 14px; color: #b9b4a2; }
.stage { position: relative; flex: 1; min-height: 0; display: grid; place-items: center; overflow: hidden; touch-action: none; border-top: 2px solid #c9c4b0; border-bottom: 2px solid #c9c4b0; }
.stage img { max-width: 100%; max-height: 100%; object-fit: contain; opacity: 0; transition: opacity 0.12s; user-select: none; -webkit-user-drag: none; }
.stage img.on { opacity: 1; }
.ahead { position: absolute; width: 1px; height: 1px; opacity: 0; pointer-events: none; }
/* The load bar in the viewer's own dark colours. */
.wait { position: absolute; width: 132px; --panel: #23221e; --edge: #ece8d8; }
.trouble { position: absolute; margin: 0 24px; font-size: 15px; line-height: 1.4; text-align: center; color: #b9b4a2; }
.acts { flex: none; display: flex; padding: 8px 6px 6px; }
.acts button { flex: 1; height: 58px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 3px; font: inherit; font-size: 12px; font-weight: 700; color: #ece8d8; background: none; border: 0; }
.acts .bad { color: #ff7a7a; }
</style>
