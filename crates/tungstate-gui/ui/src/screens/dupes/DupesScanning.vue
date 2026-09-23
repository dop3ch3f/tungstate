<!-- A scan, while it runs. The first folder pass in the project that reports
     progress: a drive takes minutes, and a window that says nothing for
     minutes looks hung. -->
<script setup lang="ts">
import { useDupes } from "../../state/useDupes";
import { bytes, shortPath } from "../../lib/format";
import Button from "../../ui/Button.vue";

const d = useDupes();
</script>

<template>
  <div class="dz-run">
    <p class="dz-what">
      Looking through <span class="path">{{ shortPath(d.root.value ?? "") }}</span>
    </p>
    <p class="dz-counts num" v-if="d.progress.value">
      {{ d.progress.value.looked }} file(s) checked, {{ d.progress.value.read }} read,
      {{ d.progress.value.recalled }} already known, {{ bytes(d.progress.value.bytes) }} read.
    </p>
    <p class="dz-counts" v-else>Walking the folder…</p>
    <p class="path dz-at" v-if="d.progress.value">{{ d.progress.value.path }}</p>

    <div class="dz-act">
      <Button look="danger" :disabled="d.stopping.value" @click="d.stop()">
        {{ d.stopping.value ? "Stopping…" : "Stop" }}
      </Button>
      <span class="dz-safe">Nothing is being changed. Stopping loses only the time.</span>
    </div>
  </div>
</template>

<style scoped>
.dz-run { display: flex; flex-direction: column; gap: var(--s3); padding-top: var(--s5); }
.dz-what { font-size: var(--body); margin: 0; }
.dz-counts { font-size: var(--small); color: var(--text-quiet); margin: 0; }
.dz-at { color: var(--text-faint); margin: 0; }
.dz-act { display: flex; align-items: center; gap: var(--s3); margin-top: var(--s4); }
.dz-safe { font-size: var(--small); color: var(--text-faint); }
</style>
