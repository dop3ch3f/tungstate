<!-- Make a sync: a name, two or more folders, which way things go, and how it
     behaves. Checked by the same code the command line uses, so a refusal here
     is the refusal `tungstate sync add` would give, in plainer words. -->
<script setup lang="ts">
import { computed, onMounted, ref, shallowRef } from "vue";
import { connections, folders, syncs } from "../../engine/commands";
import type { Connection, SyncSettingsForm } from "../../engine/types";
import { WAYS, reason } from "../../lib/syncwords";
import { shortPath } from "../../lib/format";
import { useSync } from "../../state/useSync";
import Button from "../../ui/Button.vue";
import Field from "../../ui/Field.vue";
import Notice from "../../ui/Notice.vue";
import SyncSettings from "./SyncSettings.vue";

const s = useSync();

interface Place {
  end: string;
  name: string;
}

const name = ref("");
const places = ref<Place[]>([]);
const direction = ref("all");
const anchor = ref<string | null>(null);
const settings = ref<SyncSettingsForm>({
  exact: false,
  on_conflict: "quarantine",
  on_remove: "set-aside",
  verify: "hash",
  cooldown_secs: 30,
  first_check: "full",
  launch: "no",
});
const saved = shallowRef<Connection[]>([]);
const onConnection = ref("");
const inside = ref("");
const problem = ref<string | null>(null);
const making = ref(false);

onMounted(async () => {
  try {
    saved.value = await connections.list();
  } catch {
    // Folders on this Mac still work without the list.
  }
});

/** What a member will be called if the name is left empty: its folder's last
 *  part, or its connection's name. The engine chooses the same. */
const called = (end: string) =>
  /^[A-Za-z0-9_-]{2,}:/.test(end) ? end.split(":")[0] : end.split(/[\\/]/).filter(Boolean).pop() ?? end;

async function addHere() {
  const picked = await folders.pick();
  if (picked) places.value = [...places.value, { end: picked, name: "" }];
}

function addRemote() {
  if (!onConnection.value) return;
  const end = `${onConnection.value}:${inside.value.trim().replace(/^\/+/, "")}`;
  places.value = [...places.value, { end, name: "" }];
  inside.value = "";
}

function drop(index: number) {
  places.value = places.value.filter((_, i) => i !== index);
}

const named = computed(() => places.value.map((p) => p.name.trim() || called(p.end)));
const oneWay = computed(() => direction.value !== "all");
const ready = computed(() => name.value.trim() && places.value.length >= 2 && !making.value);

async function make() {
  making.value = true;
  problem.value = null;
  try {
    const made = await syncs.make({
      name: name.value.trim(),
      ends: places.value.map((p) => p.end),
      names: places.value.map((p) => p.name.trim()),
      direction: direction.value,
      anchor: oneWay.value ? anchor.value ?? named.value[0] ?? null : null,
      settings: settings.value,
    });
    await s.load();
    s.open(made);
  } catch (e) {
    problem.value = reason(e);
  } finally {
    making.value = false;
  }
}
</script>

<template>
  <div class="mk">
    <h2 class="mk-title">New sync</h2>

    <Field label="Name" note="How you will find it: capcut, photos, the NAS archive.">
      <input v-model="name" placeholder="capcut" />
    </Field>

    <section class="mk-block">
      <h2>Folders</h2>
      <ol class="mk-places" v-if="places.length">
        <li v-for="(place, index) in places" :key="place.end" class="mk-place">
          <span class="path mk-end">{{ shortPath(place.end) }}</span>
          <input class="mk-name" v-model="place.name" :placeholder="called(place.end)" aria-label="What to call it" />
          <Button look="link" @click="drop(index)">Remove</Button>
        </li>
      </ol>
      <p class="mk-none" v-else>Two or more: a folder on this Mac, a mounted drive, or a place on a saved connection.</p>
      <div class="mk-add">
        <Button @click="addHere()">Add a folder on this Mac…</Button>
        <template v-if="saved.length">
          <span class="mk-or">or on</span>
          <select v-model="onConnection" aria-label="Connection">
            <option value="">a connection…</option>
            <option v-for="c in saved" :key="c.name" :value="c.name">{{ c.name }}</option>
          </select>
          <input v-if="onConnection" v-model="inside" placeholder="folder on it, e.g. capcut" aria-label="Folder on the connection" />
          <Button v-if="onConnection" @click="addRemote()">Add</Button>
        </template>
      </div>
    </section>

    <section class="mk-block">
      <h2>Which way</h2>
      <div class="mk-ways">
        <label v-for="w in WAYS" :key="w.id" class="mk-way" :class="{ 'mk-on': direction === w.id }">
          <input type="radio" name="way" :value="w.id" v-model="direction" />
          <b>{{ w.label }}</b>
          <span>{{ w.line }}</span>
        </label>
      </div>
      <label class="mk-anchor" v-if="oneWay && places.length">
        <span>{{ direction === "push" ? "Which folder sends" : "Which folder everything comes into" }}</span>
        <select :value="anchor ?? named[0]" @change="anchor = ($event.target as HTMLSelectElement).value">
          <option v-for="n in named" :key="n" :value="n">{{ n }}</option>
        </select>
      </label>
    </section>

    <SyncSettings v-model="settings" />

    <Notice tone="bad" v-if="problem">{{ problem }}</Notice>

    <footer class="mk-foot">
      <Button look="primary" :disabled="!ready" @click="make()">{{ making ? "Making it…" : "Make the sync" }}</Button>
      <Button look="link" @click="s.back()">Cancel</Button>
      <span class="mk-hint" v-if="!ready && !making">{{ !name.trim() ? "Give it a name first." : "Add two folders or more first." }}</span>
      <span class="mk-hint" v-else>Nothing moves yet. Its first run is previewed before anything is copied.</span>
    </footer>
  </div>
</template>

<style scoped>
.mk { display: flex; flex-direction: column; gap: var(--s4); max-width: 820px; }
.mk-title { font-size: var(--title); font-weight: 700; margin: 0; }
.mk-block h2 { font-size: var(--small); font-weight: 700; margin: 0 0 var(--s2); }
.mk-places { list-style: none; margin: 0 0 var(--s2); padding: 0; border-top: var(--bw) solid var(--rule); }
.mk-place { display: grid; grid-template-columns: minmax(0, 1fr) 180px auto; gap: var(--s3); align-items: center; padding: 6px 0; border-bottom: var(--bw) solid var(--rule); }
.mk-end { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--small); }
.mk-name { font-size: var(--small); }
.mk-none { font-size: var(--small); color: var(--text-faint); margin: 0 0 var(--s2); }
.mk-add { display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; }
.mk-or { font-size: var(--small); color: var(--text-faint); }
.mk-ways { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--s2); }
.mk-way {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: var(--s3) var(--s3) var(--s3) 34px;
  background: var(--panel);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius);
  font-size: var(--small);
  cursor: pointer;
}
.mk-way input { position: absolute; left: var(--s3); top: 14px; margin: 0; accent-color: var(--control); }
.mk-way span { font-size: var(--fine); color: var(--text-faint); line-height: 1.45; }
.mk-on { box-shadow: inset 0 0 0 var(--bw) var(--control), var(--lift); }
.mk-anchor { display: flex; align-items: center; gap: var(--s2); margin-top: var(--s3); font-size: var(--small); }
.mk-foot { display: flex; align-items: center; gap: var(--s3); flex-wrap: wrap; }
.mk-hint { font-size: var(--fine); color: var(--text-faint); }
:global([data-theme="retro"] .mk-way.mk-on) { box-shadow: inset 0 0 0 var(--bw) var(--edge), var(--lift); }
@media (max-width: 760px) { .mk-ways { grid-template-columns: 1fr; } }
</style>
