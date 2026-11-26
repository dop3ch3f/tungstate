<!-- The phone app: one screen at a time over a tab bar. Its own frame rather
     than the desktop's rail and windows; the engine, stores and theme are the
     desktop's own. Matches docs/mobile/approved-files/. -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch, type Component } from "vue";
import { onBackButtonPress, exit } from "@tauri-apps/api/app";
import { attachTransferStream, catchUp, useTransfer } from "../state/useTransfer";
import { phone } from "../engine/commands";
import { addStep, stepBack } from "./lib/back";
import { isAndroid } from "./platform";
import { usePhoneNav, type Tab } from "./nav";
import TabBar from "./TabBar.vue";
import FilesScreen from "./screens/FilesScreen.vue";
import ConnectionsScreen from "./screens/ConnectionsScreen.vue";
import DupesScreen from "./screens/DupesScreen.vue";
import OrganiseScreen from "./screens/OrganiseScreen.vue";
import FreeUpScreen from "./screens/FreeUpScreen.vue";
import FreeingScreen from "./screens/FreeingScreen.vue";
import MovingScreen from "./files/MovingScreen.vue";
import MovingBar from "./files/MovingBar.vue";
import PButton from "./ui/PButton.vue";
import PSheet from "./ui/PSheet.vue";
import { useFreeUp } from "./state/useFreeUp";
import { useClipboard } from "./state/useClipboard";
import { useWindows } from "./state/useWindows";
import { nameOf } from "./lib/paths";

const nav = usePhoneNav();
const run = useFreeUp();
const board = useClipboard();
const w = useWindows();
const t = useTransfer();
// A Free up run takes the whole screen, tabs and all, until it is done with.
const freeing = computed(() => run.phase.value !== "idle");

/** "Keep browsing" tucks a running move behind a bar; a new one shows again. */
const tucked = ref(false);
watch(() => board.started.value?.job, () => (tucked.value = false));
const moving = computed(() => board.started.value !== null && !tucked.value);

const SCREENS: Record<Tab, Component> = {
  files: FilesScreen,
  free: FreeUpScreen,
  dupes: DupesScreen,
  organise: OrganiseScreen,
  connections: ConnectionsScreen,
};
const shown = computed(() => SCREENS[nav.tab.value]);

// Ticking files puts the actions where the tabs were, as ES does.
const selecting = computed(() => w.selecting.value && (nav.tab.value === "files" || nav.tab.value === "free"));

// While the keyboard is up the page is much shorter than the screen; the tab
// bar is tucked away then, as phone apps do, so a form keeps its room.
const typing = ref(false);
const measure = () => (typing.value = window.innerHeight < window.screen.height * 0.7);
// The bars' real heights, where the phone says; the CSS fallbacks are for a
// phone that does not, and for the harness.
const insets = ref<{ top: number; bottom: number } | null>(null);
async function readInsets() {
  try {
    const got = await phone.insets();
    if (got.top > 0 || got.bottom > 0) insets.value = got;
  } catch {
    // Not a phone, or an old plugin: the CSS defaults stand.
  }
}

// Android's back button: whatever is on screen steps back first (lib/back);
// at the bottom of it all another tab goes to Files, and Files home leaves
// the app on a second press, so one stray swipe never closes it.
const leaving = ref(false);
let leaveTimer: ReturnType<typeof setTimeout> | undefined;
const removeBase = addStep(() => {
  if (nav.tab.value !== "files") return nav.go("files"), true;
  return false;
});
let unlistenBack: { unregister: () => Promise<void> } | undefined;
// Shown again after the phone had it in the background or locked: the
// engine's events may have gone unseen meanwhile, so take its own account.
// A copy the phone closed Tungstate on shows up then too, with Resume.
function shownAgain() {
  if (document.visibilityState !== "visible") return;
  void catchUp();
  if (!t.running.value) void t.refreshStranded();
}

onMounted(async () => {
  window.addEventListener("resize", measure);
  document.addEventListener("visibilitychange", shownAgain);
  await attachTransferStream();
  void catchUp();
  void t.refreshStranded();
  void readInsets();
  if (!isAndroid()) return;
  try {
    unlistenBack = await onBackButtonPress(() => {
      if (stepBack()) return;
      if (leaving.value) return void exit(0);
      leaving.value = true;
      clearTimeout(leaveTimer);
      leaveTimer = setTimeout(() => (leaving.value = false), 2000);
    });
  } catch {
    // Outside Tauri (the harness): there is no back button to take over.
  }
});
onUnmounted(() => {
  window.removeEventListener("resize", measure);
  document.removeEventListener("visibilitychange", shownAgain);
  removeBase();
  void unlistenBack?.unregister();
});

/** The same file is already there, byte for byte: may the original go? */
const sameEverywhere = ref(false);
async function identical(remove: boolean) {
  await t.answerIdentical(remove, sameEverywhere.value);
  sameEverywhere.value = false;
}
</script>

<template>
  <div
    class="phone"
    :class="{ android: isAndroid() }"
    :style="insets ? { '--top': `${insets.top}px`, '--bottom': `${insets.bottom}px` } : undefined"
  >
    <main class="stage">
      <FreeingScreen v-if="freeing" />
      <MovingScreen v-else-if="moving" @hide="tucked = true" />
      <component :is="shown" v-else :key="nav.tab.value" :tab="nav.tab.value" />
    </main>
    <footer v-show="!typing && !freeing && !moving && !selecting" class="foot">
      <MovingBar v-if="board.started.value" @show="tucked = false" />
      <TabBar />
    </footer>
    <div v-show="selecting || moving" class="inset"></div>
    <p v-if="leaving" class="toast" role="status">Press back again to leave</p>

    <PSheet
      v-if="t.identical.value"
      :title="`${nameOf(t.identical.value.path)} is already there`"
      says="Both copies were read and they match. Remove the one on this phone?"
      @close="identical(false)"
    >
      <label class="all"><input v-model="sameEverywhere" type="checkbox" /> Do the same for any others</label>
      <PButton @press="identical(true)">Remove it from this phone</PButton>
      <PButton quiet @press="identical(false)">Keep it here too</PButton>
    </PSheet>
  </div>
</template>

<style scoped>
.phone {
  /* iOS reports its notch and home bar; Android draws under a status bar it
     reports nothing for, so it gets its status bar and gesture bar by hand. */
  --top: env(safe-area-inset-top, 0px);
  --bottom: env(safe-area-inset-bottom, 0px);
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--field);
  color: var(--text);
  overflow: hidden;
  -webkit-user-select: none;
  user-select: none;
  -webkit-tap-highlight-color: transparent;
}
.android { --top: 40px; --bottom: 22px; }
/* The desktop theme draws a thick scrollbar; on a phone, scrolling is its own cue. */
.phone :deep(*) { scrollbar-width: none; }
.phone :deep(*::-webkit-scrollbar) { display: none; }
.stage { flex: 1; min-height: 0; display: flex; flex-direction: column; padding-top: var(--top); }
.foot { flex: none; padding-bottom: var(--bottom); background: var(--rail); }
/* With the tabs away, the gesture bar still needs its room below. */
.inset { flex: none; height: var(--bottom); background: var(--panel); }
.toast { position: fixed; left: 50%; bottom: calc(var(--bottom) + 90px); z-index: 9; transform: translateX(-50%); margin: 0; padding: 10px 16px; font-size: 15px; font-weight: 700; white-space: nowrap; color: var(--panel); background: var(--edge); border-radius: 8px; }
.all { display: flex; align-items: center; gap: 10px; min-height: 44px; font-size: 16px; font-weight: 700; }
.all input { width: 24px; height: 24px; }
</style>
