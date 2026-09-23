<!-- One flow: point at something, watch it look, decide, and see what it did. -->
<script setup lang="ts">
import { onMounted } from "vue";
import { useDupes } from "../../state/useDupes";
import { bytes } from "../../lib/format";
import DupesStart from "./DupesStart.vue";
import DupesScanning from "./DupesScanning.vue";
import DupesFound from "./DupesFound.vue";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import Working from "../../ui/Working.vue";
import Tile from "../../ui/Tile.vue";

const d = useDupes();
onMounted(() => d.loadAction());
</script>

<template>
  <div class="dz">
    <div class="dz-inner">
      <div class="head"><Tile of="dupes" :size="26" /><h1>Duplicates</h1></div>

      <Notice v-if="d.putBackCount.value !== null">
        Put {{ d.putBackCount.value }} file(s) back where they were.
      </Notice>
      <DupesStart v-if="d.phase.value === 'start'" />
      <DupesScanning v-else-if="d.phase.value === 'scanning'" />
      <DupesFound v-else-if="d.phase.value === 'found'" />

      <Working
        v-else-if="d.phase.value === 'clearing'"
        what="Checking every byte, then clearing"
        note="nothing moves until the check agrees"
      />

      <div class="dz-done" v-else-if="d.cleared.value">
        <Notice>
          {{ d.cleared.value.files }} file(s)
          {{ d.cleared.value.reversible ? "set aside" : "sent to the Trash" }}.
          {{ bytes(d.cleared.value.bytes) }} reclaimed.
        </Notice>
        <Notice tone="hold" v-if="d.cleared.value.dropped">
          {{ d.cleared.value.dropped }} group(s) were not identical after all, and were left alone.
        </Notice>
        <Notice tone="bad" v-for="failure in d.cleared.value.failed" :key="failure">
          {{ failure }}
        </Notice>
        <p class="dz-back" v-if="d.cleared.value.reversible">
          Every file it set aside can go back where it was.
        </p>
        <p class="dz-back" v-else>
          These are in your Trash. Only the Finder can put them back.
        </p>
        <div class="dz-again">
          <Button
            v-if="d.cleared.value.reversible"
            look="primary"
            @click="d.putBack()"
          >Put it back</Button>
          <Button @click="d.again()">Look somewhere else</Button>
          <Button @click="d.root.value && d.look(d.root.value)">Scan again</Button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dz { position: absolute; inset: 0; overflow-y: auto; scrollbar-gutter: stable; }
.dz-inner { padding: var(--win-pad); display: flex; flex-direction: column; gap: var(--s4); min-height: 100%; }
.head { display: flex; align-items: center; gap: var(--s3); }
h1 { font-size: var(--display); font-weight: 700; margin: 0; letter-spacing: -0.01em; }
.dz-done { display: flex; flex-direction: column; gap: var(--s3); }
.dz-back { font-size: var(--small); color: var(--text-quiet); margin: 0; }
.dz-again { display: flex; gap: var(--s2); margin-top: var(--s3); }
</style>
