<!-- Places that are not this machine. Each row can be checked, because a
     connection that lists but refuses files is the failure that matters. -->
<script setup lang="ts">
import { onMounted, ref, shallowRef } from "vue";
import { connections } from "../../engine/commands";
import type { Connection, Probe } from "../../engine/types";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import Empty from "../../ui/Empty.vue";

const all = shallowRef<Connection[]>([]);
const probes = ref<Record<string, Probe | string | "checking">>({});
const problem = ref<string | null>(null);

async function load() {
  try {
    all.value = await connections.list();
  } catch (e) {
    problem.value = String(e);
  }
}
onMounted(load);

async function check(name: string) {
  probes.value = { ...probes.value, [name]: "checking" };
  try {
    const got = await connections.test(name);
    probes.value = { ...probes.value, [name]: got };
  } catch (e) {
    probes.value = { ...probes.value, [name]: String(e) };
  }
}

/** Listing and writing are different permissions, and only the second is what
 *  a drain needs. A root that lists happily and refuses every file is the
 *  case this sentence exists for. */
function verdict(got: Probe | string | "checking" | undefined): string {
  if (got === undefined) return "not checked";
  if (got === "checking") return "checking…";
  if (typeof got === "string") return got;
  if (got.accepts_files === false) return `lists ${got.entries} things but refuses to take a file`;
  return `reachable, ${got.entries} things in ${got.root}`;
}

async function remove(name: string) {
  const answer = await ask({
    title: `Remove ${name}?`,
    why: "Any saved pair that uses it will stop working until it is set up again.",
    choices: [
      { id: "no", label: "Keep it" },
      { id: "yes", label: "Remove", look: "danger" },
    ],
  });
  if (answer.id !== "yes") return;
  try {
    await connections.remove(name);
    await load();
  } catch (e) {
    problem.value = String(e);
  }
}
</script>

<template>
  <div class="cx-wrap">
    <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
    <Empty v-if="!all.length" line="No connections yet. You need one to reach a NAS over FTP. A volume you have mounted in Finder needs none." />
    <div class="cx-row" v-for="c in all" :key="c.name">
      <div class="cx-who">
        <span class="cx-name">{{ c.name }}</span>
        <span class="cx-where">
          {{ c.scheme }}<template v-if="c.host">://{{ c.host }}<template v-if="c.port">:{{ c.port }}</template></template>
        </span>
      </div>
      <div class="cx-say">
        <span class="cx-state">{{ verdict(probes[c.name]) }}</span>
        <span class="cx-warn" v-if="!c.encrypted">sends your password in the clear</span>
        <span class="cx-warn" v-if="c.rootless">{{ c.rootless }}</span>
      </div>
      <div class="cx-do">
        <Button @click="check(c.name)">Check</Button>
        <Button look="danger" @click="remove(c.name)">Remove</Button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.cx-wrap { display: flex; flex-direction: column; gap: var(--s3); }
.cx-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1.4fr) auto;
  gap: var(--s4);
  align-items: start;
  padding: var(--s3) 0;
  border-bottom: 1px solid var(--edge);
}
.cx-who { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.cx-name { font-weight: 600; font-size: var(--small); }
.cx-where { font-size: var(--fine); color: var(--text-faint); }
.cx-say { display: flex; flex-direction: column; gap: 2px; font-size: var(--fine); }
.cx-state { color: var(--text-quiet); }
.cx-warn { color: var(--hold); }
.cx-do { display: flex; gap: var(--s2); }
</style>
