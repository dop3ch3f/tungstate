<!-- A newer version, what changed in it, and the one button that installs it.
     The restart waits for anything writing files, and says what. -->
<script setup lang="ts">
import { computed } from "vue";
import { useUpdate, type Waiting } from "../state/useUpdate";
import { bytes } from "../lib/format";
import Button from "../ui/Button.vue";
import ActionBar from "../ui/ActionBar.vue";
import Notice from "../ui/Notice.vue";
import Sheet from "../ui/Sheet.vue";

const u = useUpdate();
const WAITING: Record<Waiting, string> = {
  transfer: "Waiting for the transfer to finish.",
  sync: "Waiting for the sync to finish.",
  tidy: "Waiting for the tidy to finish.",
  clearing: "Waiting for the duplicates to be cleared.",
};
const doing = computed(() => {
  switch (u.step.value) {
    case "fetching":
      return u.total.value ? `Downloading, ${bytes(u.got.value)} of ${bytes(u.total.value)}.` : "Downloading.";
    case "waiting":
      return u.waitingFor.value ? WAITING[u.waitingFor.value] : "";
    case "installing":
      return "Installing. Tungstate opens again by itself.";
    default:
      return "";
  }
});
</script>

<template>
  <Sheet v-if="u.showing.value && u.ready.value" of="settings" :title="`Tungstate ${u.ready.value.version}`" @dismiss="u.showing.value = false">
    <!-- Neutral, wherever in the window it opens: the sheet is teleported to
         the frame, so the scope is set in here. -->
    <div data-half="settings">
    <p class="up-lede">Tungstate restarts into the new version. Anything running finishes first.</p>
    <pre class="up-notes" v-if="u.ready.value.body">{{ u.ready.value.body }}</pre>
    <p class="up-doing" v-if="doing">{{ doing }}</p>
    <Notice tone="bad" v-if="u.problem.value">{{ u.problem.value }}</Notice>
    <ActionBar pinned>
      <Button @click="u.showing.value = false">Later</Button>
      <Button look="primary" :busy="u.step.value !== 'idle'" @click="u.apply()">Update and restart</Button>
    </ActionBar>
    </div>
  </Sheet>
</template>

<style scoped>
.up-lede { font-size: var(--small); color: var(--text-quiet); margin: 0 0 var(--s3); }
.up-notes {
  font: inherit;
  font-size: var(--small);
  white-space: pre-wrap;
  max-height: 260px;
  overflow-y: auto;
  margin: 0 0 var(--s3);
  padding: var(--s3);
  background: var(--surface);
  border: 1px solid var(--rule);
  border-radius: var(--radius);
}
.up-doing { font-size: var(--small); margin: 0 0 var(--s3); }
</style>
