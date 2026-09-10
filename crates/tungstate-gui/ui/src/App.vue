<script setup lang="ts">
import { ref, onMounted } from "vue";
import LinksView from "./components/LinksView.vue";
import RunView from "./components/RunView.vue";
import ActivityView from "./components/ActivityView.vue";
import QuarantineView from "./components/QuarantineView.vue";

type Mode = "links" | "run" | "activity" | "quarantine";

const mode = ref<Mode>("links");
const running = ref<string | null>(null);

// Switching to the transfer view is the app's job, not the user's: starting a
// drain and then having to find where it went would be a poor introduction.
function started(name: string) {
  running.value = name;
  mode.value = "run";
}

onMounted(() => { document.title = "Tungstate"; });
</script>

<template>
  <div class="shell">
    <aside class="rail">
      <div class="wordmark">tung<span>state</span></div>
      <nav>
        <button :aria-current="mode === 'links'" @click="mode = 'links'">Folders</button>
        <button :aria-current="mode === 'run'" @click="mode = 'run'">Transfer</button>
        <button :aria-current="mode === 'activity'" @click="mode = 'activity'">Activity</button>
        <button :aria-current="mode === 'quarantine'" @click="mode = 'quarantine'">Quarantine</button>
      </nav>
      <div class="foot">Nothing is deleted until its copy is verified.</div>
    </aside>

    <main class="pane">
      <LinksView v-if="mode === 'links'" @run="started" />
      <RunView v-else-if="mode === 'run'" :link="running" />
      <ActivityView v-else-if="mode === 'activity'" />
      <QuarantineView v-else />
    </main>
  </div>
</template>
