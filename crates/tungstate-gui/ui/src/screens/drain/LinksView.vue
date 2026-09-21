<!-- Saved pairs: where from, where to, and the six settings in plain words. -->
<script setup lang="ts">
import { onMounted, ref, shallowRef } from "vue";
import { links } from "../../engine/commands";
import { CHOICES, type Link } from "../../engine/types";
import { duration } from "../../lib/format";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import Empty from "../../ui/Empty.vue";

const emit = defineEmits<{ ran: [] }>();
const all = shallowRef<Link[]>([]);
const problem = ref<string | null>(null);

async function load() {
  try {
    all.value = await links.list();
  } catch (e) {
    problem.value = String(e);
  }
}
onMounted(load);

/** The prose for a stored token, from the same table the dialog offers. */
function says(field: keyof typeof CHOICES, token: string): string {
  const found = (CHOICES[field] as readonly (readonly [string, string])[]).find(([v]) => v === token);
  return found ? found[1] : token;
}

async function run(name: string) {
  try {
    await links.run(name);
    emit("ran");
  } catch (e) {
    problem.value = String(e);
  }
}

async function remove(link: Link) {
  const answer = await ask({
    title: `Remove ${link.name}?`,
    why: "If this pair has moved anything, it is retired rather than deleted: its history stays, so a file can still be traced. If it has never run, it goes for good.",
    choices: [
      { id: "no", label: "Keep it" },
      { id: "yes", label: "Remove", look: "danger" },
    ],
  });
  if (answer.id !== "yes") return;
  try {
    await links.remove(link.name);
    await load();
  } catch (e) {
    problem.value = String(e);
  }
}
</script>

<template>
  <div class="lk-wrap">
    <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
    <Empty v-if="!all.length" line="No saved pairs yet. Tick some files in the browser and name the pair when you send them." />
    <div class="lk-row" v-for="link in all" :key="link.name">
      <div class="lk-who">
        <span class="lk-name">{{ link.name }}</span>
        <span class="path lk-ends">{{ link.source }} → {{ link.destination }}</span>
      </div>
      <ul class="lk-how">
        <li>{{ says("source_policy", link.source_policy) }}</li>
        <li>{{ says("verify", link.verify) }}</li>
        <li>{{ says("order", link.order) }}</li>
        <li>{{ says("on_conflict", link.on_conflict) }}</li>
        <li v-if="link.cooldown_secs">waits {{ duration(link.cooldown_secs) }} after a file is written</li>
      </ul>
      <div class="lk-do">
        <Button @click="run(link.name)">Run</Button>
        <Button look="danger" @click="remove(link)">Remove</Button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.lk-wrap { display: flex; flex-direction: column; gap: var(--s3); }
.lk-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) auto;
  gap: var(--s4);
  align-items: start;
  padding: var(--s3) 0;
  border-bottom: 1px solid var(--edge);
}
.lk-who { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.lk-name { font-weight: 600; font-size: var(--small); }
.lk-ends { color: var(--text-faint); }
.lk-how { list-style: none; margin: 0; padding: 0; font-size: var(--fine); color: var(--text-quiet); }
.lk-how li { padding: 1px 0; }
.lk-do { display: flex; gap: var(--s2); }
</style>
