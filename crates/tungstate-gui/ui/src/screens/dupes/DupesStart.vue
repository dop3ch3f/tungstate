<!-- Point at something. Any folder, governed or not, or a connection: finding
     duplicates has nothing to do with rules. -->
<script setup lang="ts">
import { onMounted, ref, shallowRef } from "vue";
import { useDupes } from "../../state/useDupes";
import { folders, transfers } from "../../engine/commands";
import type { Place } from "../../engine/types";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import Empty from "../../ui/Empty.vue";

const d = useDupes();
const places = shallowRef<Place[]>([]);
const chosen = ref("");

onMounted(async () => {
  try {
    places.value = await transfers.places();
  } catch {
    // The list is a convenience; the picker still works without it.
  }
});

async function point() {
  const picked = await folders.pick();
  if (picked) await d.look(picked);
}
</script>

<template>
  <div class="dz-column">
    <p class="dz-intro">
      Files that are the same file, whatever they are called. Nothing is read
      in full unless it has to be, and nothing moves until you say so.
    </p>

    <div class="dz-pick">
      <Button look="primary" @click="point()">Point at a folder…</Button>
      <label class="dz-go">
        <span class="dz-lbl">or somewhere saved</span>
        <select v-model="chosen" @change="chosen && d.look(chosen)">
          <option value="">Go to…</option>
          <option v-for="place in places" :key="place.path" :value="place.path">
            {{ place.label }}
          </option>
        </select>
      </label>
    </div>

    <Notice tone="bad" v-if="d.problem.value">{{ d.problem.value }}</Notice>
    <Empty
      art="no-folders"
      line="A whole drive is allowed and will take a while. Downloads and Movies are where the copies usually are."
    />
  </div>
</template>

<style scoped>
.dz-column { display: flex; flex-direction: column; gap: var(--s4); }
.dz-intro { font-size: var(--body); color: var(--text-quiet); margin: 0; max-width: 70ch; line-height: 1.55; }
.dz-pick { display: flex; align-items: flex-end; gap: var(--s4); flex-wrap: wrap; }
.dz-go { display: flex; flex-direction: column; gap: 5px; }
.dz-lbl { font-size: var(--fine); color: var(--text-faint); }
</style>
