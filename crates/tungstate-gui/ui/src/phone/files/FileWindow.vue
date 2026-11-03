<!-- One file window, as ES File Explorer has them: a folder on this phone or
     a connection, or one category's files, as a list or a grid. Long-press
     to tick; the tab bar then gives way to Copy · Move · Share · More · Delete.
     Matches docs/mobile/approved-files/ (folder, select, clip, moveto).

     Copy and Move go on the clipboard, which rides along while you browse to
     where they go; with two windows open, the other window is one tap. -->
<script setup lang="ts">
import { computed, nextTick, onMounted, ref, shallowRef, watch } from "vue";
import { files, phone, transfers } from "../../engine/commands";
import type { Entry } from "../../engine/types";
import { bytes, plural, when } from "../../lib/format";
import Pixels from "../../ui/Pixels.vue";
import { artOf, CATEGORY_LABEL, tintOf } from "../lib/look";
import { connectionOf, crumbs, nameOf, parentOf, placeName, topOf } from "../lib/paths";
import { useClipboard, type Op } from "../state/useClipboard";
import { useStorage } from "../state/useStorage";
import { useWindows, type Place, type Sort } from "../state/useWindows";
import PButton from "../ui/PButton.vue";
import ClipBar from "./ClipBar.vue";
import PSheet from "../ui/PSheet.vue";
import PTile from "../ui/PTile.vue";

const w = useWindows();
const store = useStorage();
const board = useClipboard();

const entries = shallowRef<Entry[]>([]);
const loading = ref(false);
const trouble = ref<string | null>(null);
const said = ref<string | null>(null);
const search = ref<string | null>(null);
type Sheet = "sort" | "more" | "pickmore" | "name" | "delete" | "move";
const sheet = ref<Sheet | null>(null);
/** The name sheet: making a folder, or renaming the one ticked. */
const naming = ref<{ mode: "new" | "rename"; value: string } | null>(null);
const busy = ref(false);
/** What Copy or Move was pressed for, while the two-window sheet is up. */
const pending = ref<Op>("move");

const place = computed(() => w.win.value.place);
const folder = computed(() => (place.value.kind === "folder" ? place.value.path : null));
const remote = computed(() => folder.value !== null && connectionOf(folder.value) !== null);

// Answers can come back out of order when someone taps through folders
// quickly; only the latest one is drawn.
let asked = 0;
async function load(at: Place) {
  const mine = ++asked;
  loading.value = true;
  trouble.value = null;
  try {
    let got: Entry[] = [];
    if (at.kind === "folder") {
      got = (await transfers.browse(at.path)).entries;
      store.visited(at.path);
    } else if (at.kind === "category") {
      got = (await phone.categoryFiles(at.category)).map((f) => ({ name: f.name, path: f.path, is_dir: false, size: f.size, modified: f.modifiedMs }));
    }
    if (mine === asked) entries.value = got;
  } catch (e) {
    if (mine === asked) {
      entries.value = [];
      trouble.value = String(e);
    }
  } finally {
    if (mine === asked) loading.value = false;
  }
}
watch(place, (at) => {
  search.value = null;
  said.value = null;
  entries.value = [];
  void load(at);
}, { immediate: true, deep: true });
const reload = () => load(place.value);
// Opened straight on a folder (from Connections, or Recent after a restart),
// the volumes' names for the chips may not be known yet.
onMounted(() => {
  if (!store.volumes.value.length) void store.load();
});

const SORTS: { id: Sort; label: string }[] = [
  { id: "newest", label: "Newest first" },
  { id: "oldest", label: "Oldest first" },
  { id: "name", label: "Name" },
  { id: "largest", label: "Largest first" },
];
const ORDER: Record<Sort, (a: Entry, b: Entry) => number> = {
  newest: (a, b) => (b.modified ?? 0) - (a.modified ?? 0),
  oldest: (a, b) => (a.modified ?? Infinity) - (b.modified ?? Infinity),
  name: (a, b) => a.name.localeCompare(b.name, undefined, { numeric: true }),
  largest: (a, b) => b.size - a.size,
};
/** Folders first, as every file manager lists them, then the chosen order. */
const shown = computed(() => {
  const q = search.value?.trim().toLowerCase();
  const list = q ? entries.value.filter((e) => e.name.toLowerCase().includes(q)) : [...entries.value];
  const by = ORDER[w.win.value.sort];
  return list.sort((a, b) => Number(b.is_dir) - Number(a.is_dir) || by(a, b) || a.name.localeCompare(b.name));
});
const fileBytes = computed(() => entries.value.reduce((n, e) => n + (e.is_dir ? 0 : e.size), 0));

const top = computed(() => (folder.value ? topOf(folder.value, store.tops.value) : null));
const atTop = computed(() => !!folder.value && !!top.value && folder.value.replace(/\/+$/, "") === top.value.path.replace(/\/+$/, ""));
const title = computed(() => {
  if (place.value.kind === "category") return CATEGORY_LABEL[place.value.category];
  if (!folder.value) return "Files";
  return atTop.value ? top.value!.label : nameOf(folder.value);
});
const chips = computed(() => (folder.value ? crumbs(folder.value, store.tops.value) : []));

function back() {
  if (place.value.kind === "folder" && !atTop.value) {
    const up = parentOf(place.value.path);
    if (up) return w.go({ kind: "folder", path: up });
  }
  w.go({ kind: "home" });
}

// ── ticking ──────────────────────────────────────────────────────────────
const picked = computed(() => w.win.value.picked);
const pickedEntries = computed(() => entries.value.filter((e) => picked.value.has(e.path)));
const pickedBytes = computed(() => pickedEntries.value.reduce((n, e) => n + (e.is_dir ? 0 : e.size), 0));

/** "3 photos", "2 videos", "4 items": the plainest true word for the lot. */
function what(list: Entry[]): string {
  const kinds = new Set(list.map((e) => (e.is_dir ? "folder" : artOf(e))));
  const one = kinds.size === 1 ? [...kinds][0] : null;
  const word = one === "photo" ? "photo" : one === "video" ? "video" : one === "folder" ? "folder" : "item";
  return plural(list.length, word);
}

// Long-press starts ticking. A held finger fires `contextmenu` on Android's
// web view; the timer is for the harness and for web views that do not.
let held: ReturnType<typeof setTimeout> | undefined;
let heldFired = false;
function press(e: Entry) {
  heldFired = false;
  clearTimeout(held);
  held = setTimeout(() => hold(e), 450);
}
function release() {
  clearTimeout(held);
}
function hold(e: Entry) {
  heldFired = true;
  if (!picked.value.has(e.path)) w.toggle(e.path);
  navigator.vibrate?.(10);
}
async function tap(e: Entry) {
  if (heldFired) return void (heldFired = false);
  if (w.selecting.value) return w.toggle(e.path);
  if (e.is_dir) return w.go({ kind: "folder", path: e.path });
  // A file on a connection would have to come down first; for now a tap
  // ticks it, ready for Copy or Move.
  if (connectionOf(e.path) !== null) return w.toggle(e.path);
  try {
    await phone.open(e.path);
  } catch (err) {
    said.value = String(err);
  }
}

// ── actions ──────────────────────────────────────────────────────────────
/** The other window's folder, when there is one to send to. */
const otherIndex = computed(() => (w.wins.value.length > 1 ? 1 - w.active.value : null));
const otherFolder = computed(() => {
  const i = otherIndex.value;
  const at = i === null ? null : w.wins.value[i]!.place;
  return at?.kind === "folder" ? at.path : null;
});

function startClip(op: Op) {
  const list = pickedEntries.value;
  board.hold({ op, what: what(list), bytes: pickedBytes.value, from: title.value, paths: list.map((e) => e.path) });
  w.clearPicks();
}

function copyOrMove(op: Op) {
  pending.value = op;
  if (otherIndex.value !== null) sheet.value = "move";
  else startClip(op);
}

async function sendToOther() {
  const to = otherFolder.value;
  if (!to) return;
  sheet.value = null;
  startClip(pending.value);
  await pasteInto(to);
}

async function pasteInto(to: string) {
  busy.value = true;
  try {
    await board.paste(to, placeName(to, store.tops.value));
  } catch (e) {
    said.value = e instanceof Error ? e.message : String(e);
  } finally {
    busy.value = false;
  }
}

async function share() {
  const list = pickedEntries.value.filter((e) => !e.is_dir);
  if (!list.length || list.some((e) => connectionOf(e.path) !== null)) {
    said.value = "Share sends files on this phone. Move or copy these here first.";
    return;
  }
  try {
    await phone.share(list.map((e) => e.path));
  } catch (e) {
    said.value = String(e);
  }
}

function askName(mode: "new" | "rename") {
  sheet.value = "name";
  naming.value = { mode, value: mode === "rename" ? pickedEntries.value[0]?.name ?? "" : "" };
  void nextTick(() => document.querySelector<HTMLInputElement>(".name-input")?.focus());
}

// The name field takes every keystroke, composition included: Android's
// keyboard composes whole words, and `v-model` waits for the word to end, so
// pressing Make folder mid-word saved half a name.
async function saveName() {
  const n = naming.value;
  if (!n || busy.value) return;
  busy.value = true;
  try {
    if (n.mode === "new" && folder.value) await files.makeFolder(folder.value, n.value);
    else if (n.mode === "rename" && pickedEntries.value[0]) await files.rename(pickedEntries.value[0].path, n.value);
    sheet.value = null;
    naming.value = null;
    w.clearPicks();
    await reload();
  } catch (e) {
    said.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function remove() {
  busy.value = true;
  try {
    const gone = await files.remove(pickedEntries.value.map((e) => e.path));
    said.value = `Deleted ${plural(gone, "file or folder", "files and folders")}.`;
    w.clearPicks();
  } catch (e) {
    said.value = String(e);
  } finally {
    busy.value = false;
    sheet.value = null;
    await reload();
  }
}

// Swipe sideways on the list to change window, once there are two.
let from: { x: number; y: number } | null = null;
function swipeStart(e: PointerEvent) {
  from = { x: e.clientX, y: e.clientY };
}
function swipeEnd(e: PointerEvent) {
  if (!from || w.wins.value.length < 2) return;
  const dx = e.clientX - from.x;
  if (Math.abs(dx) > 70 && Math.abs(e.clientY - from.y) < 40) w.switchTo(w.active.value + (dx < 0 ? 1 : -1));
  from = null;
}

const placeLabel = computed(() => (folder.value ? (connectionOf(folder.value) ?? top.value?.label ?? "this phone") : "this phone"));
const sub = (e: Entry) => [e.is_dir ? null : bytes(e.size), e.modified ? when(e.modified) : null].filter(Boolean).join(" · ") || (e.is_dir ? "Folder" : "");
</script>

<template>
  <section class="window">
    <!-- Header: ticking, or where this window is. -->
    <div v-if="w.selecting.value" class="head">
      <button class="iconbtn" aria-label="Stop selecting" @click="w.clearPicks()">✕</button>
      <div class="grow">
        <b class="count">{{ picked.size }} selected</b>
        <span v-if="pickedBytes" class="quiet">{{ bytes(pickedBytes) }}</span>
      </div>
      <button class="smallbtn" @click="w.pickAll(shown.map((e) => e.path))">Select all</button>
    </div>
    <div v-else class="head">
      <button class="back" aria-label="Back" @click="back">‹</button>
      <h1 class="grow">{{ title }}</h1>
      <button v-if="board.held.value && folder" class="txtbtn" @click="askName('new')"><Pixels of="new-folder" :size="18" />New folder</button>
      <template v-else>
        <button class="iconbtn" aria-label="Search" @click="search = search === null ? '' : null"><Pixels of="search" :size="22" /></button>
        <button class="iconbtn" aria-label="More" @click="sheet = 'more'"><Pixels of="more" :size="22" /></button>
      </template>
    </div>

    <input v-if="search !== null" :value="search" class="search" @input="search = ($event.target as HTMLInputElement).value" type="search" placeholder="Find in this folder" aria-label="Find in this folder" />
    <div v-if="chips.length" class="crumbs">
      <template v-for="(c, i) in chips" :key="c.path">
        <span v-if="i" class="sep" aria-hidden="true">›</span>
        <button class="chip" @click="w.go({ kind: 'folder', path: c.path })">{{ c.label }}</button>
      </template>
    </div>
    <div class="bar">
      <button class="sortbtn" @click="sheet = 'sort'">{{ SORTS.find((s) => s.id === w.win.value.sort)!.label }} ▾</button>
      <span class="quiet">{{ loading ? "Opening…" : fileBytes ? `${plural(entries.length, "item")} · ${bytes(fileBytes)}` : plural(entries.length, "item") }}</span>
      <div class="seg" role="group" aria-label="View">
        <button :class="{ on: w.win.value.view === 'list' }" aria-label="List" @click="w.setView('list')"><Pixels of="list" :size="18" /></button>
        <button :class="{ on: w.win.value.view === 'grid' }" aria-label="Grid" @click="w.setView('grid')"><Pixels of="grid" :size="18" /></button>
      </div>
    </div>

    <div class="body" @pointerdown="swipeStart" @pointerup="swipeEnd">
      <p v-if="said" class="line">{{ said }}</p>
      <p v-if="trouble" class="line bad">{{ trouble }}</p>
      <p v-else-if="!loading && !shown.length" class="line">{{ search ? "Nothing here has that in its name." : place.kind === "category" ? "Nothing of this kind on this phone." : "This folder is empty." }}</p>
      <div v-if="shown.length" :class="w.win.value.view === 'grid' ? 'grid' : 'list'">
        <button
          v-for="e in shown"
          :key="e.path"
          class="entry"
          :class="{ sel: picked.has(e.path) }"
          @click="tap(e)"
          @pointerdown="press(e)"
          @pointerup="release"
          @pointerleave="release"
          @contextmenu.prevent="hold(e)"
        >
          <span v-if="w.selecting.value" class="check" :class="{ on: picked.has(e.path) }" aria-hidden="true">✓</span>
          <PTile :art="artOf(e)" :tint="tintOf(e)" :size="w.win.value.view === 'grid' ? 64 : 44" />
          <span class="text">
            <b>{{ e.name }}</b>
            <span class="quiet">{{ sub(e) }}</span>
          </span>
          <span v-if="e.is_dir && !w.selecting.value && w.win.value.view === 'list'" class="chev" aria-hidden="true">›</span>
        </button>
      </div>
    </div>

    <!-- The clipboard rides along while you browse to where it goes. -->
    <ClipBar v-if="!w.selecting.value" :folder="folder" :busy="busy" @paste="folder && pasteInto(folder)" />

    <!-- Ticking: the actions take the tab bar's place. -->
    <nav v-if="w.selecting.value" class="actions">
      <button @click="copyOrMove('copy')"><Pixels of="copy" :size="24" /><span>Copy</span></button>
      <button @click="copyOrMove('move')"><Pixels of="move" :size="24" /><span>Move</span></button>
      <button @click="share"><Pixels of="share" :size="24" /><span>Share</span></button>
      <button @click="sheet = 'pickmore'"><Pixels of="more" :size="24" /><span>More</span></button>
      <button class="danger" @click="sheet = 'delete'"><Pixels of="trash" :size="24" /><span>Delete</span></button>
    </nav>

    <!-- Sheets. -->
    <PSheet v-if="sheet === 'sort'" title="Sort" @close="sheet = null">
      <div class="options">
        <button v-for="s in SORTS" :key="s.id" class="option" :class="{ on: w.win.value.sort === s.id }" @click="w.win.value.sort = s.id; sheet = null">{{ s.label }}</button>
      </div>
    </PSheet>
    <PSheet v-if="sheet === 'more'" :title="title" @close="sheet = null">
      <div class="options">
        <button v-if="folder" class="option" @click="askName('new')">New folder</button>
        <button v-if="w.wins.value.length < w.max" class="option" @click="sheet = null; w.second()">Open a second window</button>
        <button v-else class="option" @click="sheet = null; w.close(w.active.value)">Close this window</button>
        <button class="option" @click="sheet = null; reload()">Refresh</button>
      </div>
    </PSheet>
    <PSheet v-if="sheet === 'pickmore'" :title="`${picked.size} selected`" @close="sheet = null">
      <div class="options">
        <button v-if="picked.size === 1" class="option" @click="askName('rename')">Rename</button>
        <div class="detail">
          <b>{{ pickedBytes ? `${what(pickedEntries)} · ${bytes(pickedBytes)}` : what(pickedEntries) }}</b>
          <span class="quiet">In {{ [...chips.map((c) => c.label), title].join(" › ") }}</span>
        </div>
      </div>
    </PSheet>
    <PSheet v-if="sheet === 'name' && naming" :title="naming.mode === 'new' ? 'New folder' : 'Rename'" @close="sheet = null">
      <input :value="naming.value" class="name-input" @input="naming.value = ($event.target as HTMLInputElement).value" :aria-label="naming.mode === 'new' ? 'Folder name' : 'New name'" @keydown.enter="saveName" />
      <PButton :busy="busy" :disabled="!naming.value.trim()" @press="saveName">{{ naming.mode === "new" ? "Make folder" : "Rename" }}</PButton>
    </PSheet>
    <PSheet
      v-if="sheet === 'delete'"
      :title="`Delete ${what(pickedEntries)}?`"
      :says="`There is no bin on ${placeLabel}, so this cannot be undone.`"
      @close="sheet = null"
    >
      <PButton tint="var(--bad)" :busy="busy" @press="remove">Delete {{ what(pickedEntries) }}</PButton>
      <PButton quiet @press="sheet = null">{{ picked.size === 1 ? "Keep it" : "Keep them" }}</PButton>
    </PSheet>
    <PSheet
      v-if="sheet === 'move'"
      :title="`${pending === 'move' ? 'Move' : 'Copy'} ${what(pickedEntries)}`"
      :says="pending === 'move' ? 'Each is sent, read back and checked before the original is removed.' : 'Each copy is read back and checked.'"
      @close="sheet = null"
    >
      <button class="opt main" :disabled="!otherFolder" @click="sendToOther">
        <span class="num">{{ (otherIndex ?? 0) + 1 }}</span>
        <span class="grow"><b>{{ pending === "move" ? "Move" : "Copy" }} to window {{ (otherIndex ?? 0) + 1 }}</b><span class="q">{{ otherFolder ? placeName(otherFolder, store.tops.value) : "Open a folder there first" }}</span></span>
        <span class="chev" aria-hidden="true">›</span>
      </button>
      <button class="opt" @click="sheet = null; startClip(pending)">
        <PTile art="clipboard" tint="var(--panel)" :size="40" />
        <span class="grow"><b>Choose another place</b><span class="q">Browse anywhere, then tap {{ pending === "move" ? "Move here" : "Copy here" }}</span></span>
        <span class="chev" aria-hidden="true">›</span>
      </button>
      <PButton quiet @press="sheet = null">Cancel</PButton>
    </PSheet>
  </section>
</template>

<style scoped>
.window { flex: 1; min-height: 0; display: flex; flex-direction: column; }
.head { flex: none; display: flex; align-items: center; gap: 10px; padding: 6px 20px 10px; }
.grow { flex: 1; min-width: 0; }
h1 { margin: 0; font-size: 32px; line-height: 1.08; font-weight: 800; letter-spacing: -0.6px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.count { display: block; font-size: 22px; font-weight: 800; }
.quiet { font-size: 14px; color: var(--text-quiet); }
.back { flex: none; width: 44px; height: 44px; margin-left: -12px; font: inherit; font-size: 30px; font-weight: 800; color: var(--text); background: none; border: 0; }
.iconbtn, .txtbtn, .smallbtn {
  flex: none; display: flex; align-items: center; justify-content: center; gap: 6px; height: 44px;
  font: inherit; font-weight: 800; color: var(--text); background: var(--panel);
  border: 2px solid var(--edge); border-radius: 8px; box-shadow: 3px 3px 0 var(--edge);
}
.iconbtn { width: 44px; font-size: 20px; }
.txtbtn, .smallbtn { padding: 0 12px; font-size: 15px; white-space: nowrap; }
.iconbtn:active, .txtbtn:active, .smallbtn:active { transform: translate(3px, 3px); box-shadow: none; }
.search { flex: none; margin: 0 20px 10px; height: 44px; padding: 0 12px; font: inherit; font-size: 16px; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.crumbs { flex: none; display: flex; align-items: center; gap: 6px; padding: 0 20px 10px; overflow-x: auto; white-space: nowrap; scrollbar-width: none; }
.chip { flex: none; height: 40px; padding: 0 12px; font: inherit; font-size: 14px; font-weight: 700; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 6px; }
.sep { font-weight: 900; }
.bar { flex: none; display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 0 20px 10px; }
.sortbtn { height: 44px; padding: 0; font: inherit; font-size: 15px; font-weight: 800; color: var(--text); background: none; border: 0; }
.seg { display: flex; overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.seg button { width: 44px; height: 40px; display: grid; place-items: center; color: var(--text); background: none; border: 0; border-right: 2px solid var(--edge); }
.seg button:last-child { border-right: 0; }
.seg button.on { color: var(--panel); background: var(--edge); }
.body { flex: 1; min-height: 0; overflow-y: auto; padding: 0 20px 16px; -webkit-overflow-scrolling: touch; }
.line { margin: 4px 2px 12px; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.bad { color: var(--bad); font-weight: 700; }
.list { background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; overflow: hidden; }
.entry {
  display: flex; align-items: center; gap: 12px; width: 100%; min-height: 64px; padding: 9px 14px;
  font: inherit; text-align: left; color: var(--text); background: var(--panel);
  border: 0; border-top: 2px solid var(--edge);
  -webkit-touch-callout: none;
}
.list .entry:first-child { border-top: 0; }
.entry:active { background: var(--surface-hover); }
.entry.sel { background: color-mix(in srgb, var(--tint-drain) 14%, var(--panel)); }
.text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.text b { font-size: 16px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.chev { flex: none; font-size: 22px; font-weight: 800; }
.check { flex: none; width: 30px; height: 30px; display: grid; place-items: center; font-weight: 900; color: transparent; background: var(--panel); border: 2px solid var(--edge); border-radius: 6px; }
.check.on { color: #ffffff; background: var(--tint-drain); }
/* The grid: three across, the name under each, ticks in the corner. */
.grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 8px; }
.grid .entry { position: relative; flex-direction: column; justify-content: flex-start; gap: 6px; min-height: 0; padding: 10px 6px; text-align: center; border: 2px solid var(--edge); border-radius: 8px; }
.grid .text b { font-size: 13px; }
.grid .text .quiet { font-size: 12px; }
.grid .check { position: absolute; top: 6px; left: 6px; width: 26px; height: 26px; }
.actions { flex: none; display: flex; padding: 8px 4px 6px; border-top: 2px solid var(--edge); background: var(--panel); }
.actions button { flex: 1; height: 58px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 3px; font: inherit; font-size: 12px; font-weight: 700; color: var(--text); background: none; border: 0; }
.actions .danger { color: var(--bad); }
.options { display: flex; flex-direction: column; overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.option { min-height: 52px; padding: 0 16px; font: inherit; font-size: 16px; font-weight: 700; text-align: left; color: var(--text); background: none; border: 0; border-top: 2px solid var(--edge); }
.option:first-child { border-top: 0; }
.option.on { font-weight: 800; background: var(--surface-hover); }
.option.on::after { content: " ✓"; }
.detail { display: flex; flex-direction: column; gap: 4px; padding: 14px 16px; border-top: 2px solid var(--edge); }
.detail:first-child { border-top: 0; }
.name-input { height: 52px; padding: 0 14px; font: inherit; font-size: 17px; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.opt { display: flex; align-items: center; gap: 12px; min-height: 72px; padding: 12px 14px; font: inherit; text-align: left; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 4px 4px 0 var(--edge); }
.opt b { display: block; font-size: 16px; }
.opt .q { font-size: 14px; color: var(--text-quiet); }
.opt.main { color: #ffffff; background: var(--tint-drain); }
.opt.main .q { color: #dfe6ff; }
.opt:disabled { opacity: 0.55; }
.num { width: 40px; height: 40px; flex: none; display: grid; place-items: center; font-size: 16px; font-weight: 900; color: var(--text); background: #ffffff; border: 2px solid var(--edge); border-radius: 8px; }
</style>
