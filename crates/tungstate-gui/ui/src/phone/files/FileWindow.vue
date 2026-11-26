<!-- One file window, as ES File Explorer has them: a folder on this phone or
     a connection, or one category's files, in one of three views (List,
     Detailed list, Tiles), each with previews. Long-press to tick; the tab
     bar then gives way to Copy · Move · Share · More · Delete. A tap opens:
     a folder, the viewer for a photo, or the phone's own app for the rest
     (fetched first when it is on a connection).

     Copy and Move go on the clipboard, which rides along while you browse to
     where they go; with other tabs open on folders, each is one tap. -->
<script lang="ts">
import { makeShelf } from "../lib/shelf";
import type { Entry } from "../../engine/types";

// Outlives any one window, so a folder opened in either window, or before a
// tab change, comes back at once.
const shelf = makeShelf<Entry>();
</script>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, shallowRef, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { files, phone, transfers } from "../../engine/commands";
import type { Fetching, StoredFile } from "../../engine/types";
import { bytes, kind, plural, when } from "../../lib/format";
import Pixels from "../../ui/Pixels.vue";
import { useBack } from "../lib/back";
import { artOf, CATEGORY_LABEL, isPicture } from "../lib/look";
import { connectionOf, crumbs, nameOf, parentOf, placeName, topOf } from "../lib/paths";
import { rowsInView, useScroll } from "../lib/virtual";
import { useClipboard, type Op } from "../state/useClipboard";
import { useStorage } from "../state/useStorage";
import { useWindows, type Place, type Sort, type View } from "../state/useWindows";
import PButton from "../ui/PButton.vue";
import PLoadBar from "../ui/PLoadBar.vue";
import ClipBar from "./ClipBar.vue";
import Viewer from "./Viewer.vue";
import PSheet from "../ui/PSheet.vue";
import PThumb from "../ui/PThumb.vue";

const w = useWindows();
const store = useStorage();
const board = useClipboard();

const entries = shallowRef<Entry[]>([]);
const loading = ref(false);
const trouble = ref<string | null>(null);
const said = ref<string | null>(null);
const search = ref<string | null>(null);
type Sheet = "sort" | "more" | "pickmore" | "name" | "delete" | "move" | "info";
const sheet = ref<Sheet | null>(null);
/** The name sheet: making a folder, or renaming the one ticked. */
const naming = ref<{ mode: "new" | "rename"; value: string } | null>(null);
const busy = ref(false);
/** What Copy or Move was pressed for, while the two-window sheet is up. */
const pending = ref<Op>("move");
/** The one file the Info and Delete sheets are about, from the viewer. */
const about = ref<Entry | null>(null);
/** The viewer, open on these photos at this one. */
const viewing = ref<{ items: Entry[]; at: number } | null>(null);
/** A file on a connection being fetched to open, and how far it has come. */
const fetching = ref<{ path: string; done: number; total: number } | null>(null);

const place = computed(() => w.win.value.place);
const folder = computed(() => (place.value.kind === "folder" ? place.value.path : null));
const placeKey = (at: Place) => (at.kind === "folder" ? at.path : at.kind === "category" ? `category:${at.category}` : "home");

const box = ref<HTMLElement | null>(null);
const scroll = useScroll(box);

/** Set once a listing has kept someone waiting, so a quick folder does not
 *  flash placeholders on its way in. */
const slow = ref(false);
let slowTimer: ReturnType<typeof setTimeout> | undefined;
/** A category's first screenful, with room for a quick flick. */
const FIRST = 200;
const shelfKey = (at: Place) => `${placeKey(at)}|${w.hidden.value ? "all" : "plain"}`;

function restoreScroll(at: Place) {
  void nextTick(() => {
    // Back where this folder was left, as ES does when you come back up.
    if (box.value) box.value.scrollTop = w.win.value.scrolled.get(placeKey(at)) ?? 0;
    scroll.measure();
  });
}

// Answers can come back out of order when someone taps through folders
// quickly; only the latest one is drawn.
let asked = 0;
/** Lists `at`. Arriving somewhere draws its last listing at once, if there is
 *  one, and the fresh one replaces it; `again` refreshes what is on screen
 *  in place, keeping the scroll where it is. */
async function load(at: Place, again = false) {
  const mine = ++asked;
  let drewEarly = false;
  const key = shelfKey(at);
  const known = again ? undefined : shelf.get(key);
  if (!again) {
    entries.value = known ?? [];
    if (known) restoreScroll(at);
  }
  loading.value = true;
  slow.value = false;
  clearTimeout(slowTimer);
  slowTimer = setTimeout(() => {
    if (mine === asked) slow.value = true;
  }, 150);
  trouble.value = null;
  try {
    let got: Entry[] = [];
    if (at.kind === "folder") {
      got = (await transfers.browse(at.path, w.hidden.value)).entries;
      store.visited(at.path);
    } else if (at.kind === "category") {
      const asEntries = (list: StoredFile[]) => list.map((f): Entry => ({ name: f.name, path: f.path, is_dir: false, size: f.size, modified: f.modifiedMs }));
      // A big category (Downloads can hold 100k files) takes seconds to list
      // in full; its first screenful, in the order on screen, comes in about
      // one. Draw that, then swap in the whole list behind it.
      let whole: Entry[] | null = null;
      if (!known && !again) {
        const early = asEntries(await phone.categoryFiles(at.category, FIRST, w.win.value.sort));
        // Gone elsewhere already: the full list would only hold up the next.
        if (mine !== asked) return;
        // Fewer than asked for is every one of them.
        if (early.length < FIRST) whole = early;
        else if (mine === asked) {
          entries.value = early;
          slow.value = true;
          drewEarly = true;
          restoreScroll(at);
        }
      }
      got = whole ?? asEntries(await phone.categoryFiles(at.category));
    }
    if (mine === asked) {
      entries.value = got;
      shelf.put(key, got);
    }
  } catch (e) {
    if (mine === asked) {
      entries.value = [];
      trouble.value = String(e);
    }
  } finally {
    if (mine === asked) {
      loading.value = false;
      slow.value = false;
      clearTimeout(slowTimer);
      // Already placed: moving the scroll now would yank someone mid-flick.
      if (again || known || drewEarly) void nextTick(scroll.measure);
      else restoreScroll(at);
    }
  }
}
watch(place, (at, was) => {
  if (was && box.value) w.win.value.scrolled.set(placeKey(was), box.value.scrollTop);
  search.value = null;
  said.value = null;
  void load(at);
}, { immediate: true, deep: true });
watch(w.hidden, () => void load(place.value));
const reload = () => load(place.value, true);
// Opened straight on a folder (from Connections, or Recent after a restart),
// the volumes' names for the chips may not be known yet.
let unlisten: UnlistenFn | undefined;
onMounted(async () => {
  if (!store.volumes.value.length) void store.load();
  unlisten = await listen<Fetching>("fetch://progress", (e) => {
    if (fetching.value?.path === e.payload.path) fetching.value = { ...e.payload };
  });
});
onUnmounted(() => {
  unlisten?.();
  clearTimeout(slowTimer);
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
// Android's back: end selection first, then go up a level.
useBack(() => {
  if (w.selecting.value) return w.clearPicks(), true;
  if (search.value !== null) return (search.value = null), true;
  back();
  return true;
});

// ── the three views, drawing only what is on screen ─────────────────────
const VIEWS: { id: View; label: string; art: "list" | "detail" | "grid" }[] = [
  { id: "list", label: "List", art: "list" },
  { id: "detailed", label: "Detailed list", art: "detail" },
  { id: "tiles", label: "Tiles", art: "grid" },
];
const view = computed(() => w.win.value.view);
const columns = computed(() => (view.value === "tiles" ? 3 : 1));
/** Each view's row height. A tile is its square plus two lines of name. */
const rowHeight = computed(() => {
  if (view.value === "list") return 56;
  if (view.value === "detailed") return 88;
  const cell = (scroll.width.value - 40 - 2 * 10) / 3;
  return Math.round(cell + 50);
});
const rows = computed(() => Math.ceil(shown.value.length / columns.value));
const span = computed(() => rowsInView(scroll.top.value, scroll.height.value, rowHeight.value, rows.value));
const drawn = computed(() => shown.value.slice(span.value[0] * columns.value, span.value[1] * columns.value));
/** Placeholder rows while the first listing of a place is on its way: one
 *  screenful, in the shape of the view, so the page does not sit empty. */
const ghosts = computed(() => Math.min(30, Math.ceil(scroll.height.value / rowHeight.value) * columns.value));
const ghostWidth = (i: number) => `${[72, 54, 84, 62, 46, 78][i % 6]}%`;

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
  if (pulled) return;
  if (w.selecting.value) return w.toggle(e.path);
  if (e.is_dir) return w.go({ kind: "folder", path: e.path });
  if (isPicture(e.name)) {
    const photos = shown.value.filter((x) => !x.is_dir && isPicture(x.name));
    viewing.value = { items: photos, at: Math.max(0, photos.findIndex((x) => x.path === e.path)) };
    return;
  }
  await openElsewhere(e);
}

/** Hands a file to the phone's app for its kind, fetching it first from a
 *  connection, with the row showing how far the fetch has come. */
async function openElsewhere(e: Entry) {
  try {
    let local = e.path;
    if (connectionOf(e.path) !== null) {
      fetching.value = { path: e.path, done: 0, total: e.size };
      local = await phone.fetchCopy(e.path);
    }
    await phone.open(local);
  } catch (err) {
    said.value = String(err);
  } finally {
    fetching.value = null;
  }
}

// ── actions ──────────────────────────────────────────────────────────────
/** The other tabs open on a folder: each is somewhere to send to. */
const others = computed(() =>
  w.wins.value.flatMap((x, index) => (index !== w.active.value && x.place.kind === "folder" ? [{ index, folder: x.place.path }] : [])),
);

function startClip(op: Op, list = pickedEntries.value) {
  const sum = list.reduce((n, e) => n + (e.is_dir ? 0 : e.size), 0);
  board.hold({ op, what: what(list), bytes: sum, from: title.value, paths: list.map((e) => e.path) });
  w.clearPicks();
}

function copyOrMove(op: Op) {
  pending.value = op;
  if (others.value.length) sheet.value = "move";
  else startClip(op);
}

async function sendToOther(to: string) {
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

async function shareEntries(list: Entry[]) {
  const wanted = list.filter((e) => !e.is_dir);
  if (!wanted.length) return void (said.value = "Folders can't be shared; open one and choose its files.");
  try {
    // A file on a connection is fetched first; other apps can only read
    // files on this phone.
    const local = [];
    for (const e of wanted) local.push(connectionOf(e.path) !== null ? await phone.fetchCopy(e.path) : e.path);
    await phone.share(local);
  } catch (e) {
    said.value = String(e);
  }
}

/** Opens the search field ready to type in, or closes it. */
const searchBox = ref<HTMLInputElement | null>(null);
function toggleSearch() {
  search.value = search.value === null ? "" : null;
  if (search.value !== null) void nextTick(() => searchBox.value?.focus());
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

/** What the delete sheet is about: the one from the viewer, or the ticked. */
const doomed = computed(() => (about.value ? [about.value] : pickedEntries.value));
async function remove() {
  busy.value = true;
  const list = doomed.value;
  try {
    const gone = await files.remove(list.map((e) => e.path));
    said.value = `Deleted ${plural(gone, "file or folder", "files and folders")}.`;
    w.clearPicks();
    if (viewing.value) {
      const left = viewing.value.items.filter((x) => !list.some((d) => d.path === x.path));
      viewing.value = left.length ? { items: left, at: Math.min(viewing.value.at, left.length - 1) } : null;
    }
  } catch (e) {
    said.value = String(e);
  } finally {
    busy.value = false;
    sheet.value = null;
    about.value = null;
    await reload();
  }
}

/** From the viewer: one photo onto the clipboard, then back to the folder. */
function moveOne(e: Entry) {
  viewing.value = null;
  startClip("move", [e]);
}
function infoOf(e: Entry) {
  about.value = e;
  sheet.value = "info";
}
function deleteOne(e: Entry) {
  about.value = e;
  sheet.value = "delete";
}
function closeSheet() {
  sheet.value = null;
  about.value = null;
}

/** Info about the one ticked or the one in the viewer. */
const infoFor = computed(() => about.value ?? (pickedEntries.value.length === 1 ? pickedEntries.value[0]! : null));
const whereOf = (e: Entry) => {
  const up = parentOf(e.path);
  return up ? placeName(up, store.tops.value) : "";
};

// Swipe sideways on the list to change tab, once there are two.
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

// Pull down at the top of the list to look again, as ES does.
const pull = ref(0);
let pullFrom: number | null = null;
let pulled = false;
function touchStart(e: TouchEvent) {
  pulled = false;
  pullFrom = box.value && box.value.scrollTop <= 0 ? e.touches[0]!.clientY : null;
}
function touchMove(e: TouchEvent) {
  if (pullFrom === null) return;
  pull.value = Math.max(0, Math.min(90, (e.touches[0]!.clientY - pullFrom) * 0.5));
}
function touchEnd() {
  if (pull.value >= 60) {
    pulled = true;
    void reload();
  }
  pull.value = 0;
  pullFrom = null;
}

const placeLabel = computed(() => (folder.value ? (connectionOf(folder.value) ?? top.value?.label ?? "this phone") : "this phone"));
const sub = (e: Entry) => [e.is_dir ? null : bytes(e.size), e.modified ? when(e.modified) : null].filter(Boolean).join(" · ") || (e.is_dir ? "Folder" : "");
const kindLine = (e: Entry) => (e.is_dir ? "Folder" : [bytes(e.size), kind(e)].join(" · "));
const progress = (e: Entry) => (fetching.value?.path === e.path ? fetching.value : null);
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
        <button class="iconbtn" aria-label="Search" @click="toggleSearch"><Pixels of="search" :size="22" /></button>
        <button class="iconbtn" aria-label="More" @click="sheet = 'more'"><Pixels of="more" :size="22" /></button>
      </template>
    </div>

    <input v-if="search !== null" ref="searchBox" :value="search" class="search" @input="search = ($event.target as HTMLInputElement).value" type="search" placeholder="Find in this folder" aria-label="Find in this folder" />
    <div v-if="chips.length || folder" class="crumbs">
      <template v-for="(c, i) in chips" :key="c.path">
        <span v-if="i" class="sep" aria-hidden="true">›</span>
        <button class="chip" @click="w.go({ kind: 'folder', path: c.path })">{{ c.label }}</button>
      </template>
      <template v-if="chips.length">
        <span class="sep" aria-hidden="true">›</span>
        <span class="chip" aria-current="page">{{ title }}</span>
      </template>
    </div>
    <div class="bar">
      <button class="sortbtn" @click="sheet = 'sort'">{{ SORTS.find((s) => s.id === w.win.value.sort)!.label.split(" ")[0] }} ▾</button>
      <span v-if="loading && slow" class="count-line wait"><PLoadBar :label="`Opening ${title}`" /></span>
      <span v-else class="quiet count-line">{{ loading && !entries.length ? "" : fileBytes ? `${entries.length} · ${bytes(fileBytes)}` : plural(entries.length, "item") }}</span>
      <div class="seg" role="group" aria-label="View">
        <button v-for="v in VIEWS" :key="v.id" :class="{ on: view === v.id }" :aria-label="v.label" :aria-pressed="view === v.id" @click="w.setView(v.id)"><Pixels :of="v.art" :size="18" /></button>
      </div>
    </div>

    <div
      ref="box"
      class="body"
      @pointerdown="swipeStart"
      @pointerup="swipeEnd"
      @touchstart.passive="touchStart"
      @touchmove.passive="touchMove"
      @touchend="touchEnd"
    >
      <div class="pull" :style="{ height: `${pull}px` }" aria-hidden="true"><span v-if="pull > 20">{{ pull >= 60 ? "Let go to refresh" : "Pull to refresh" }}</span></div>
      <p v-if="said" class="line">{{ said }}</p>
      <p v-if="trouble" class="line bad">{{ trouble }}</p>
      <p v-else-if="!loading && !shown.length" class="line">{{ search ? "Nothing here has that in its name." : place.kind === "category" ? "Nothing of this kind on this phone." : "This folder is empty." }}</p>
      <div v-if="loading && slow && !entries.length" class="ghosts" :class="view" aria-hidden="true">
        <div v-for="i in ghosts" :key="i" class="ghost" :style="{ height: `${rowHeight}px` }">
          <span class="gpic" :style="view === 'tiles' ? undefined : { width: `${view === 'detailed' ? 66 : 36}px`, height: `${view === 'detailed' ? 66 : 36}px` }"></span>
          <span class="gtext">
            <span class="gline" :style="{ width: ghostWidth(i) }"></span>
            <span v-if="view === 'detailed'" class="gline short"></span>
          </span>
        </div>
      </div>
      <div v-if="shown.length" class="rows" :class="view" :style="{ height: `${rows * rowHeight}px` }">
        <div class="slice" :style="{ transform: `translateY(${span[0] * rowHeight}px)` }">
          <button
            v-for="e in drawn"
            :key="e.path"
            class="entry"
            :class="{ sel: picked.has(e.path) }"
            :style="{ height: `${rowHeight}px` }"
            @click="tap(e)"
            @pointerdown="press(e)"
            @pointerup="release"
            @pointerleave="release"
            @contextmenu.prevent="hold(e)"
          >
            <span v-if="w.selecting.value" class="check" :class="{ on: picked.has(e.path) }" aria-hidden="true">✓</span>
            <template v-if="view === 'tiles'">
              <PThumb class="pic" :name="e.name" :path="e.path" :is-dir="e.is_dir" fill />
              <b class="tname">{{ e.name }}</b>
            </template>
            <template v-else>
              <PThumb :name="e.name" :path="e.path" :is-dir="e.is_dir" :size="view === 'detailed' ? 66 : 36" />
              <span class="text">
                <b>{{ e.name }}</b>
                <template v-if="progress(e)">
                  <span class="quiet">Opening… {{ bytes(progress(e)!.done) }} of {{ bytes(progress(e)!.total) }}</span>
                  <span class="fetchbar"><i :style="{ width: `${progress(e)!.total ? (100 * progress(e)!.done) / progress(e)!.total : 5}%` }"></i></span>
                </template>
                <template v-else-if="view === 'detailed'">
                  <span class="quiet">{{ kindLine(e) }}</span>
                  <span v-if="e.modified" class="quiet">{{ when(e.modified) }}</span>
                </template>
              </span>
              <span v-if="view === 'list' && !progress(e)" class="quiet side">{{ e.is_dir ? "" : bytes(e.size) }}</span>
              <span v-if="e.is_dir && !w.selecting.value" class="chev" aria-hidden="true">›</span>
            </template>
          </button>
        </div>
      </div>
    </div>

    <!-- The clipboard rides along while you browse to where it goes. -->
    <ClipBar v-if="!w.selecting.value" :folder="folder" :busy="busy" @paste="folder && pasteInto(folder)" />

    <!-- Ticking: the actions take the tab bar's place. -->
    <nav v-if="w.selecting.value" class="actions">
      <button @click="copyOrMove('copy')"><Pixels of="copy" :size="24" /><span>Copy</span></button>
      <button @click="copyOrMove('move')"><Pixels of="move" :size="24" /><span>Move</span></button>
      <button @click="shareEntries(pickedEntries)"><Pixels of="share" :size="24" /><span>Share</span></button>
      <button @click="sheet = 'pickmore'"><Pixels of="more" :size="24" /><span>More</span></button>
      <button class="danger" @click="sheet = 'delete'"><Pixels of="trash" :size="24" /><span>Delete</span></button>
    </nav>

    <Viewer
      v-if="viewing"
      :items="viewing.items"
      :start="viewing.at"
      @close="viewing = null"
      @share="(e) => shareEntries([e])"
      @move="moveOne"
      @info="infoOf"
      @delete="deleteOne"
    />

    <!-- Sheets. -->
    <PSheet v-if="sheet === 'sort'" title="Sort" @close="sheet = null">
      <div class="options">
        <button v-for="s in SORTS" :key="s.id" class="option" :class="{ on: w.win.value.sort === s.id }" @click="w.win.value.sort = s.id; sheet = null">{{ s.label }}</button>
      </div>
    </PSheet>
    <PSheet v-if="sheet === 'more'" :title="title" @close="sheet = null">
      <div class="options">
        <button v-if="folder" class="option" @click="askName('new')">New folder</button>
        <button v-if="folder" class="option" :class="{ on: w.hidden.value }" @click="w.setHidden(!w.hidden.value); sheet = null">Show hidden files</button>
        <button class="option" @click="sheet = null; reload()">Refresh</button>
      </div>
    </PSheet>
    <PSheet v-if="sheet === 'pickmore'" :title="`${picked.size} selected`" @close="sheet = null">
      <div class="options">
        <button v-if="picked.size === 1" class="option" @click="askName('rename')">Rename</button>
        <button v-if="infoFor" class="option" @click="sheet = 'info'">Info</button>
        <button v-if="infoFor && !infoFor.is_dir" class="option" @click="sheet = null; openElsewhere(infoFor)">Open with…</button>
        <div v-if="!infoFor" class="detail">
          <b>{{ pickedBytes ? `${what(pickedEntries)} · ${bytes(pickedBytes)}` : what(pickedEntries) }}</b>
          <span class="quiet">In {{ [...chips.map((c) => c.label), title].join(" › ") }}</span>
        </div>
      </div>
    </PSheet>
    <PSheet v-if="sheet === 'info' && infoFor" :title="infoFor.name" @close="closeSheet">
      <div class="ihead">
        <PThumb :name="infoFor.name" :path="infoFor.path" :is-dir="infoFor.is_dir" :size="72" />
        <span class="quiet">{{ infoFor.is_dir ? "Folder" : kind(infoFor) }}</span>
      </div>
      <dl class="props">
        <template v-if="!infoFor.is_dir"><dt>Size</dt><dd>{{ bytes(infoFor.size) }}</dd></template>
        <template v-if="infoFor.modified"><dt>Changed</dt><dd>{{ when(infoFor.modified) }}</dd></template>
        <dt>Where</dt><dd>{{ whereOf(infoFor) }}</dd>
      </dl>
      <div class="pair">
        <PButton v-if="!about && picked.size === 1" quiet @press="askName('rename')">Rename</PButton>
        <PButton v-if="!infoFor.is_dir" quiet @press="closeSheet(); openElsewhere(infoFor)">Open with…</PButton>
      </div>
    </PSheet>
    <PSheet v-if="sheet === 'name' && naming" :title="naming.mode === 'new' ? 'New folder' : 'Rename'" @close="sheet = null">
      <input :value="naming.value" class="name-input" @input="naming.value = ($event.target as HTMLInputElement).value" :aria-label="naming.mode === 'new' ? 'Folder name' : 'New name'" @keydown.enter="saveName" />
      <PButton :busy="busy" :disabled="!naming.value.trim()" @press="saveName">{{ naming.mode === "new" ? "Make folder" : "Rename" }}</PButton>
    </PSheet>
    <PSheet
      v-if="sheet === 'delete'"
      :title="`Delete ${what(doomed)}?`"
      :says="`There is no bin on ${placeLabel}, so this cannot be undone.`"
      @close="closeSheet"
    >
      <PButton tint="var(--bad)" :busy="busy" @press="remove">Delete {{ what(doomed) }}</PButton>
      <PButton quiet @press="closeSheet">{{ doomed.length === 1 ? "Keep it" : "Keep them" }}</PButton>
    </PSheet>
    <PSheet
      v-if="sheet === 'move'"
      :title="`${pending === 'move' ? 'Move' : 'Copy'} ${what(pickedEntries)}`"
      :says="pending === 'move' ? 'Each is sent, read back and checked before the original is removed.' : 'Each copy is read back and checked.'"
      @close="sheet = null"
    >
      <button v-for="(o, i) in others" :key="o.index" class="opt" :class="{ main: i === 0 }" @click="sendToOther(o.folder)">
        <span class="num"><Pixels :of="connectionOf(o.folder) === null ? 'phone' : 'nas'" :size="20" /></span>
        <span class="grow"><b>{{ pending === "move" ? "Move" : "Copy" }} to {{ placeName(o.folder, store.tops.value) }}</b><span class="q">The tab you have open there</span></span>
        <span class="chev" aria-hidden="true">›</span>
      </button>
      <button class="opt" @click="sheet = null; startClip(pending)">
        <span class="num"><Pixels of="clipboard" :size="20" /></span>
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
.crumbs:empty { display: none; }
.chip { flex: none; height: 38px; padding: 0 12px; display: grid; place-items: center; font: inherit; font-size: 14px; font-weight: 700; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 6px; }
.chip[aria-current="page"] { color: var(--panel); background: var(--edge); }
.sep { font-weight: 900; }
.bar { flex: none; display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 0 20px 10px; }
.count-line { flex: 1; min-width: 0; text-align: center; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.sortbtn { height: 44px; padding: 0; font: inherit; font-size: 15px; font-weight: 800; color: var(--text); background: none; border: 0; white-space: nowrap; }
.seg { flex: none; display: flex; overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.seg button { width: 44px; height: 40px; display: grid; place-items: center; color: var(--text); background: none; border: 0; border-right: 2px solid var(--edge); }
.seg button:last-child { border-right: 0; }
.seg button.on { color: var(--panel); background: var(--edge); }
.body { flex: 1; min-height: 0; overflow-y: auto; padding: 0 20px 16px; -webkit-overflow-scrolling: touch; overscroll-behavior-y: contain; }
.pull { display: grid; place-items: end center; overflow: hidden; font-size: 14px; font-weight: 700; color: var(--text-quiet); }
.line { margin: 4px 2px 12px; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.bad { color: var(--bad); font-weight: 700; }
.rows { position: relative; }
.rows.list .slice, .rows.detailed .slice { background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; overflow: hidden; }
.slice { will-change: transform; }
.entry {
  display: flex; align-items: center; gap: 12px; width: 100%; padding: 0 14px; box-sizing: border-box;
  font: inherit; text-align: left; color: var(--text); background: var(--panel);
  border: 0; border-top: 2px solid var(--edge);
  -webkit-touch-callout: none;
}
.slice .entry:first-child { border-top: 0; }
.entry:active { background: var(--surface-hover); }
.entry.sel { background: color-mix(in srgb, var(--tint-drain) 14%, var(--panel)); }
.text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.text b { font-size: 16px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.list .text b { font-size: 15px; font-weight: 700; }
.side { flex: none; }
.chev { flex: none; font-size: 22px; font-weight: 800; }
.check { flex: none; width: 30px; height: 30px; display: grid; place-items: center; font-weight: 900; color: transparent; background: var(--panel); border: 2px solid var(--edge); border-radius: 6px; }
.check.on { color: #ffffff; background: var(--tint-drain); }
.count-line.wait { padding: 0 14px; }
/* Placeholders take the shape of the view they stand in for. */
.ghosts.list, .ghosts.detailed { background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; overflow: hidden; }
.ghosts.tiles { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); column-gap: 10px; }
.ghost { display: flex; align-items: center; gap: 12px; padding: 0 14px; box-sizing: border-box; border-top: 2px solid var(--edge); }
.ghost:first-child { border-top: 0; }
.tiles .ghost { flex-direction: column; align-items: stretch; justify-content: flex-start; gap: 8px; padding: 4px 0 0; border: 0; }
.gpic, .gline { flex: none; display: block; background: color-mix(in srgb, var(--text-quiet) 24%, var(--panel)); border-radius: 6px; animation: breathe 1.4s ease-in-out infinite; }
.tiles .gpic { width: 100%; aspect-ratio: 1; border-radius: 0; }
.gtext { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 10px; }
.gline { height: 12px; border-radius: 3px; }
.gline.short { width: 38% !important; height: 10px; }
@keyframes breathe { 50% { opacity: 0.45; } }
@media (prefers-reduced-motion: reduce) { .gpic, .gline { animation: none; } }
.fetchbar { display: block; height: 10px; margin-top: 4px; overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 3px; }
.fetchbar i { display: block; height: 100%; background: var(--tint-drain); }
/* Tiles: a contact sheet, three across, the name under each. */
.rows.tiles .slice { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); column-gap: 10px; }
.tiles .entry { position: relative; flex-direction: column; justify-content: flex-start; align-items: stretch; gap: 6px; padding: 4px 0 0; background: none; border: 0; }
.tiles .entry.sel { background: none; }
.tiles .entry.sel .pic { outline: 4px solid var(--tint-drain); outline-offset: -4px; }
.tname { display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; font-size: 12.5px; line-height: 1.25; word-break: break-all; }
.tiles .check { position: absolute; top: 10px; left: 6px; z-index: 1; width: 26px; height: 26px; }
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
.ihead { display: flex; align-items: center; gap: 14px; }
.props { display: grid; grid-template-columns: 96px 1fr; margin: 0; overflow: hidden; border: 2px solid var(--edge); border-radius: 8px; }
.props dt, .props dd { margin: 0; padding: 10px 12px; font-size: 15px; border-top: 2px solid var(--edge); overflow-wrap: anywhere; }
.props dt { font-weight: 800; background: var(--rail); border-right: 2px solid var(--edge); }
.props dt:first-of-type, .props dd:first-of-type { border-top: 0; }
.pair { display: flex; gap: 12px; }
.pair > * { flex: 1; }
.name-input { height: 52px; padding: 0 14px; font: inherit; font-size: 17px; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.opt { display: flex; align-items: center; gap: 12px; min-height: 72px; padding: 12px 14px; font: inherit; text-align: left; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 4px 4px 0 var(--edge); }
.opt b { display: block; font-size: 16px; }
.opt .q { font-size: 14px; color: var(--text-quiet); }
.opt.main { color: #ffffff; background: var(--tint-drain); }
.opt.main .q { color: #dfe6ff; }
.opt:disabled { opacity: 0.55; }
.num { width: 40px; height: 40px; flex: none; display: grid; place-items: center; font-size: 16px; font-weight: 900; color: var(--text); background: #ffffff; border: 2px solid var(--edge); border-radius: 8px; }
</style>
