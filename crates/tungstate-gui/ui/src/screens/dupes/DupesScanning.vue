<!-- A scan, while it runs. The first folder pass in the project that reports
     progress: a drive takes minutes, and a window that says nothing for
     minutes looks hung. -->
<script setup lang="ts">
import { useDupes } from "../../state/useDupes";
import { bytes, shortPath } from "../../lib/format";
import { files, opened, reached } from "../../lib/counts";
import Button from "../../ui/Button.vue";
import ActionBar from "../../ui/ActionBar.vue";
import Working from "../../ui/Working.vue";

const d = useDupes();
</script>

<template>
  <div class="dz-run">
    <p class="dz-what">
      <Working what="Looking through" /><span class="path">{{ shortPath(d.root.value ?? "") }}</span>
    </p>
    <!-- The count leads, as a result does elsewhere; the details sit under it. -->
    <p class="dz-big num" v-if="d.progress.value">
      {{ files(reached(d.progress.value)) }} {{ d.progress.value.stage === "alike" ? "looked at" : "checked" }}
    </p>
    <p class="dz-big" v-else>Walking the folder</p>
    <!-- Which pass, in the window's own words. The second one decodes every
         picture and every video, so it is slower by an order of magnitude and
         saying nothing about that looks like a hang. -->
    <p class="dz-counts num" v-if="d.progress.value?.stage === 'alike'">
      Looking at what things are, not just what they are made of:
      {{ opened(d.progress.value) }} opened, {{ d.progress.value.recalled }} remembered from last time.
    </p>
    <p class="dz-counts num" v-else-if="d.progress.value">
      {{ opened(d.progress.value) }} opened to compare, {{ d.progress.value.recalled }} remembered from last time.
    </p>
    <!-- No total is known until the walk ends, so the bar moves rather than fills. -->
    <div class="dz-bar" aria-hidden="true"><i></i></div>
    <p class="path dz-at" v-if="d.progress.value" :title="d.progress.value.path"><bdi>{{ d.progress.value.path }}</bdi></p>

    <ActionBar class="dz-end">
      <template #say>Nothing is being changed. Stopping loses only the time.</template>
      <Button :busy="d.stopping.value" @click="d.stop()">Stop</Button>
    </ActionBar>
  </div>
</template>

<style scoped>
.dz-run { flex: 1; display: flex; flex-direction: column; gap: var(--s3); }
.dz-what { display: flex; align-items: center; gap: var(--s2); font-size: var(--body); margin: 0; }
.dz-big { font-size: var(--hero); font-weight: 700; letter-spacing: -0.02em; line-height: 1.1; margin: var(--s2) 0 0; }
.dz-counts { font-size: var(--body); color: var(--text-quiet); margin: 0; }
.dz-bar { height: 8px; border: var(--bw) solid var(--edge); border-radius: var(--radius); background: var(--surface); overflow: hidden; max-width: 520px; }
.dz-bar i { display: block; width: 30%; height: 100%; background: var(--control); animation: dz-sweep 1.4s var(--ease) infinite; }
@keyframes dz-sweep { from { transform: translateX(-100%); } to { transform: translateX(340%); } }
/* Right-to-left so the ellipsis eats the front and the file's own name survives. */
.dz-at { color: var(--text-faint); margin: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; }
.dz-end { margin-top: auto; }
</style>
