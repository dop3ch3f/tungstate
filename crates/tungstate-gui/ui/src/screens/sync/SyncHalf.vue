<!-- The Sync section: the list, a new sync, one sync, its preview, its run.
     The way back is on every screen after the first. -->
<script setup lang="ts">
import { useSync } from "../../state/useSync";
import Button from "../../ui/Button.vue";
import Tile from "../../ui/Tile.vue";
import Working from "../../ui/Working.vue";
import SyncStart from "./SyncStart.vue";
import SyncMake from "./SyncMake.vue";
import SyncOne from "./SyncOne.vue";
import SyncPreview from "./SyncPreview.vue";
import SyncRun from "./SyncRun.vue";

const s = useSync();
</script>

<template>
  <div class="sh">
    <div class="sh-inner">
      <div class="sh-head">
        <Tile of="sync" :size="26" /><h1>Sync</h1>
        <Button
          v-if="s.phase.value !== 'list' && s.phase.value !== 'running'"
          look="link"
          class="sh-out"
          @click="s.phase.value = 'list'; s.current.value = null"
        >All syncs</Button>
      </div>
      <div class="sh-pending" v-if="s.phase.value === 'reading'">
        <Working what="Looking at every folder" note="nothing moves; a network folder takes longer" />
      </div>
      <SyncMake v-else-if="s.phase.value === 'make'" />
      <SyncOne v-else-if="s.phase.value === 'one' && s.current.value" />
      <SyncPreview v-else-if="s.phase.value === 'preview' && s.preview.value" />
      <SyncRun v-else-if="s.phase.value === 'running' || s.phase.value === 'done'" />
      <SyncStart v-else />
    </div>
  </div>
</template>

<style scoped>
.sh { position: absolute; inset: 0; overflow-y: auto; scrollbar-gutter: stable; }
.sh-inner { padding: var(--win-pad); display: flex; flex-direction: column; gap: var(--s4); min-height: 100%; }
.sh-head { display: flex; align-items: center; gap: var(--s3); }
.sh-head h1 { font-size: var(--display); font-weight: 700; margin: 0; letter-spacing: -0.01em; }
.sh-out { margin-left: auto; }
.sh-pending { display: grid; place-items: center; min-height: 300px; }
</style>
