<!-- One flow: point at something, watch it look, decide, and see what it did. -->
<script setup lang="ts">
import { onMounted } from "vue";
import { useDupes } from "../../state/useDupes";
import { bytes, plural } from "../../lib/format";
import { cleared, files, undidDuplicates } from "../../lib/counts";
import DupesStart from "./DupesStart.vue";
import DupesScanning from "./DupesScanning.vue";
import DupesFound from "./DupesFound.vue";
import Button from "../../ui/Button.vue";
import ActionBar from "../../ui/ActionBar.vue";
import Notice from "../../ui/Notice.vue";
import Working from "../../ui/Working.vue";
import Tile from "../../ui/Tile.vue";

const d = useDupes();
onMounted(() => d.loadAction());
</script>

<template>
  <div class="dz" :class="{ fills: d.phase.value === 'found' }">
    <div class="dz-inner">
      <div class="head">
        <Tile of="dupes" :size="26" /><h1>Duplicates</h1>
        <!-- A way out of a result, which otherwise only a scan or a clearing
             could leave. -->
        <Button
          v-if="d.phase.value === 'found'"
          look="link"
          class="head-out"
          @click="d.again()"
        >Look somewhere else</Button>
      </div>

      <Notice v-if="d.putBackCount.value !== null">
        Put {{ files(undidDuplicates(d.putBackCount.value)) }} back where they were.
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
        <!-- A result screen that swallows an error is a button that does
             nothing and says nothing, which is what Put it back did here. -->
        <Notice tone="bad" v-if="d.problem.value">{{ d.problem.value }}</Notice>
        <!-- The result leads, as Organize's does after a tidy. -->
        <div class="dz-result">
          <p class="dz-big num">{{ files(cleared(d.cleared.value)) }} {{ d.cleared.value.reversible ? "set aside" : "sent to the Trash" }}</p>
          <p class="dz-sub">{{ bytes(d.cleared.value.bytes) }} reclaimed.</p>
        </div>
        <Notice tone="hold" v-if="d.cleared.value.dropped">
          {{ plural(d.cleared.value.dropped, "group") }} {{ d.cleared.value.dropped === 1 ? "was" : "were" }} not identical after all, and {{ d.cleared.value.dropped === 1 ? "was" : "were" }} left alone.
        </Notice>
        <Notice tone="bad" v-for="failure in d.cleared.value.failed" :key="failure">
          {{ failure }}
        </Notice>
        <ActionBar class="dz-again">
          <template #say>
            <span v-if="d.cleared.value.reversible">Every file it set aside can go back where it was.</span>
            <span v-else>These are in your Trash. Only the Finder can put them back.</span>
          </template>
          <Button @click="d.root.value && d.look(d.root.value)">Scan again</Button>
          <!-- Undo is never the main button: a reflex click must not undo. -->
          <Button
            v-if="d.cleared.value.reversible"
            :busy="d.puttingBack()"
            @click="d.putBack()"
          >Put it back</Button>
          <Button look="primary" @click="d.again()">Look somewhere else</Button>
        </ActionBar>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dz { position: absolute; inset: 0; overflow-y: auto; scrollbar-gutter: stable; }
/* A result is three panes and a foot with the action in it. The panes scroll
   inside themselves; the screen itself must not, or the button that does the
   thing scrolls off the bottom at the window's own minimum size. */
.fills { overflow: hidden; }
.fills .dz-inner { height: 100%; }
.dz-inner { padding: var(--win-pad); display: flex; flex-direction: column; gap: var(--s4); min-height: 100%; }
.head { display: flex; align-items: center; gap: var(--s3); }
.head-out { margin-left: auto; }
h1 { font-size: var(--display); font-weight: 700; margin: 0; letter-spacing: -0.01em; }
/* Fills the window, so the bar sits at its foot rather than mid-screen. */
.dz-done { flex: 1; display: flex; flex-direction: column; gap: var(--s3); }
.dz-result { display: flex; flex-direction: column; gap: var(--s1); }
.dz-big { font-size: var(--hero); font-weight: 700; letter-spacing: -0.02em; line-height: 1.1; margin: 0; }
.dz-sub { font-size: var(--body); color: var(--text-quiet); margin: 0; }
.dz-again { margin-top: auto; }
</style>
