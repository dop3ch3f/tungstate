<!-- The phone app: one screen at a time over a tab bar. Its own frame rather
     than the desktop's rail and windows; the engine, stores and theme are the
     desktop's own. Matches docs/mobile/approved/. -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, type Component } from "vue";
import { isAndroid } from "./platform";
import { usePhoneNav, type Tab } from "./nav";
import TabBar from "./TabBar.vue";
import HomeScreen from "./screens/HomeScreen.vue";
import ConnectionsScreen from "./screens/ConnectionsScreen.vue";
import LaterScreen from "./screens/LaterScreen.vue";
import FreeUpScreen from "./screens/FreeUpScreen.vue";

const nav = usePhoneNav();

const SCREENS: Record<Tab, Component> = {
  home: HomeScreen,
  free: FreeUpScreen,
  dupes: LaterScreen,
  organise: LaterScreen,
  connections: ConnectionsScreen,
};
const shown = computed(() => SCREENS[nav.tab.value]);

// While the keyboard is up the page is much shorter than the screen; the tab
// bar is tucked away then, as phone apps do, so a form keeps its room.
const typing = ref(false);
const measure = () => (typing.value = window.innerHeight < window.screen.height * 0.7);
onMounted(() => window.addEventListener("resize", measure));
onUnmounted(() => window.removeEventListener("resize", measure));
</script>

<template>
  <div class="phone" :class="{ android: isAndroid() }">
    <main class="stage">
      <component :is="shown" :key="nav.tab.value" :tab="nav.tab.value" />
    </main>
    <footer v-show="!typing" class="foot"><TabBar /></footer>
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
.stage { flex: 1; min-height: 0; display: flex; flex-direction: column; padding-top: var(--top); }
.foot { flex: none; padding-bottom: var(--bottom); background: var(--rail); }
</style>
