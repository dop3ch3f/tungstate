<!-- One sync: its folders in a row, what it does in words, and everything that
     can be done to it. Previewing is the main thing; running only ever follows
     a preview. -->
<script setup lang="ts">
import { computed, ref, shallowRef } from "vue";
import { connections, folders, syncs } from "../../engine/commands";
import type { Connection, PastSync, SyncSettingsForm } from "../../engine/types";
import { exactlyInFull, following, reason, wayInFull, whenItRuns, CONFLICTS } from "../../lib/syncwords";
import { useSync } from "../../state/useSync";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import Field from "../../ui/Field.vue";
import Notice from "../../ui/Notice.vue";
import Sheet from "../../ui/Sheet.vue";
import SyncRuns from "./SyncRuns.vue";
import SyncSettings from "./SyncSettings.vue";
import SyncUndoneNotice from "./SyncUndoneNotice.vue";

const s = useSync();
const sync = computed(() => s.current.value!);
const runs = computed(() => s.past.value.filter((r) => r.sync === sync.value.name));

const editing = ref<SyncSettingsForm | null>(null);
const adding = ref(false);
const forgetting = ref(false);
const forgetPath = ref("");
const saved = shallowRef<Connection[]>([]);
const onConnection = ref("");
const inside = ref("");
const calledAs = ref("");
const said = ref<string | null>(null);
const problem = ref<string | null>(null);

const conflictWords = computed(() => CONFLICTS.find((c) => c.id === sync.value.on_conflict)?.label ?? sync.value.on_conflict);
const role = (member: string) => {
  if (sync.value.anchor !== member) return null;
  return sync.value.direction === "push" ? "sends" : "receives";
};

function edit() {
  const v = sync.value;
  editing.value = {
    exact: v.exact,
    on_conflict: v.on_conflict,
    on_remove: v.on_remove,
    verify: v.verify,
    cooldown_secs: v.cooldown_secs,
    first_check: v.first_check,
    launch: v.launch,
  };
}

async function saveSettings() {
  if (!editing.value) return;
  problem.value = null;
  try {
    s.current.value = await syncs.change(sync.value.name, editing.value);
    editing.value = null;
    said.value = "Saved. It applies from the next run.";
    await s.load();
  } catch (e) {
    problem.value = reason(e);
  }
}

async function openAdding() {
  adding.value = true;
  try {
    saved.value = await connections.list();
  } catch {
    // A folder on this Mac can still be added.
  }
}

async function addEnd(end: string) {
  problem.value = null;
  try {
    s.current.value = await syncs.addMember(sync.value.name, end, calledAs.value.trim() || null);
    adding.value = false;
    calledAs.value = inside.value = "";
    said.value = "Added. The next run fills it.";
    await s.load();
  } catch (e) {
    problem.value = reason(e);
  }
}

async function addHere() {
  const picked = await folders.pick();
  if (picked) await addEnd(picked);
}

async function removeMember(member: string) {
  const answer = await ask({
    title: `Take ${member} out of ${sync.value.name}?`,
    why: "Its files stay where they are. It is simply no longer kept in step.",
    choices: [
      { id: "cancel", label: "Cancel" },
      { id: "go", label: "Take it out", look: "primary" },
    ],
  });
  if (answer.id !== "go") return;
  try {
    s.current.value = await syncs.removeMember(sync.value.name, member);
    await s.load();
  } catch (e) {
    problem.value = reason(e);
  }
}

async function removeSync() {
  const answer = await ask({
    title: `Remove ${sync.value.name}?`,
    why: "Every folder's files stay where they are. Its past runs stay in History.",
    choices: [
      { id: "cancel", label: "Cancel" },
      { id: "go", label: "Remove", look: "danger" },
    ],
  });
  if (answer.id !== "go") return;
  try {
    await s.remove(sync.value.name);
  } catch (e) {
    problem.value = reason(e);
  }
}

async function forget() {
  const path = forgetPath.value.trim();
  if (!path) return;
  forgetting.value = false;
  forgetPath.value = "";
  await s.forget([path]);
}

async function putBack(run: PastSync) {
  const answer = await ask({
    title: `Put back this run of ${run.sync}?`,
    why: "Copies it made are set aside again, and anything it set aside goes back where it was, on every folder.",
    choices: [
      { id: "cancel", label: "Cancel" },
      { id: "go", label: "Put back", look: "primary" },
    ],
  });
  if (answer.id === "go") await s.putBack(run.sync, run.plan);
}
</script>

<template>
  <div class="so">
    <h2 class="so-title">{{ sync.name }}</h2>
    <Notice tone="hold" v-if="s.isHeld(sync.name)">
      Stopped keeping in step: its next run would remove files, so it waits for you.
      <Button look="link" @click="s.look()">Look at it</Button>
    </Notice>

    <ol class="so-places">
      <li v-for="member in sync.members" :key="member.name" class="so-place" :class="{ 'so-anchor': role(member.name) }">
        <b>{{ member.name }}</b>
        <span class="path so-at">{{ member.at }}</span>
        <span class="so-role" v-if="role(member.name)">{{ role(member.name) }}</span>
        <span
          class="so-status"
          :class="{ 'so-paused': s.statusOf(sync.name, member.name)?.state === 'paused' }"
          v-if="s.statusOf(sync.name, member.name)"
        >{{ following(s.statusOf(sync.name, member.name)!) }}</span>
        <button
          class="so-drop"
          v-if="sync.members.length > 2 && !role(member.name)"
          @click="removeMember(member.name)"
          :title="`Take ${member.name} out`"
        >Take out</button>
      </li>
    </ol>

    <ul class="so-says">
      <li>{{ wayInFull(sync) }}</li>
      <li>{{ exactlyInFull(sync) }}</li>
      <li>When two folders disagree: {{ conflictWords.toLowerCase() }}.</li>
      <li>{{ whenItRuns(sync) }}</li>
    </ul>

    <div class="so-acts">
      <Button look="primary" @click="s.look()">Preview a run</Button>
      <Button @click="edit()">Change settings…</Button>
      <Button @click="openAdding()">Add a folder…</Button>
      <Button @click="forgetting = true">Forget a file…</Button>
      <span class="so-gap"></span>
      <Button look="danger" @click="removeSync()">Remove this sync</Button>
    </div>

    <Notice v-if="said">{{ said }}</Notice>
    <SyncUndoneNotice />
    <Notice tone="bad" v-if="problem || s.problem.value">{{ problem ?? s.problem.value }}</Notice>

    <SyncRuns single :title="`Runs of ${sync.name}`" :runs="runs" :busy="s.busy.value" @put-back="putBack" />

    <Sheet v-if="editing" wide of="sync" :title="`Settings for ${sync.name}`" @dismiss="editing = null">
      <SyncSettings v-model="editing" />
      <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
      <footer class="so-foot">
        <Button look="primary" @click="saveSettings()">Save</Button>
        <Button look="link" @click="editing = null">Cancel</Button>
      </footer>
    </Sheet>

    <Sheet v-if="adding" of="sync" :title="`Add a folder to ${sync.name}`" @dismiss="adding = false">
      <Field label="What to call it" note="Leave it empty to use the folder's own name.">
        <input v-model="calledAs" />
      </Field>
      <div class="so-add">
        <Button @click="addHere()">A folder on this Mac…</Button>
        <template v-if="saved.length">
          <select v-model="onConnection" aria-label="Connection">
            <option value="">or on a connection…</option>
            <option v-for="c in saved" :key="c.name" :value="c.name">{{ c.name }}</option>
          </select>
          <input v-if="onConnection" v-model="inside" placeholder="folder on it" aria-label="Folder on the connection" />
          <Button v-if="onConnection" @click="addEnd(`${onConnection}:${inside.trim().replace(/^\/+/, '')}`)">Add</Button>
        </template>
      </div>
      <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
    </Sheet>

    <Sheet v-if="forgetting" of="sync" title="Forget a file" @dismiss="forgetting = false">
      <p class="so-why">
        Takes it off every folder, and it does not come back. Without Deletions
        too, a file deleted by hand is copied back; this is how to say you meant it.
        You see what would go before anything does.
      </p>
      <Field label="File or folder" note="As the preview lists it, e.g. exports/old.mp4 or a whole folder.">
        <input v-model="forgetPath" placeholder="exports/old.mp4" @keydown.enter="forget()" />
      </Field>
      <footer class="so-foot">
        <Button look="primary" :disabled="!forgetPath.trim()" @click="forget()">Show what would go</Button>
        <Button look="link" @click="forgetting = false">Cancel</Button>
      </footer>
    </Sheet>
  </div>
</template>

<style scoped>
.so { display: flex; flex-direction: column; gap: var(--s4); }
.so-title { font-size: var(--title); font-weight: 700; margin: 0; }
.so-places { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: var(--s3); }
.so-place {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: var(--s3);
  background: var(--panel);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius-lg);
  box-shadow: var(--lift);
  font-size: var(--small);
  min-width: 0;
}
.so-anchor { box-shadow: inset 0 0 0 var(--bw) var(--control), var(--lift); }
.so-at { font-size: var(--fine); color: var(--text-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.so-role { align-self: flex-start; font-size: var(--fine); font-weight: 700; color: var(--control-ink); background: var(--control); border-radius: var(--radius); padding: 1px 6px; }
.so-status { font-size: var(--fine); color: var(--text-faint); }
.so-paused { color: var(--hold); }
.so-drop { align-self: flex-start; font: inherit; font-size: var(--fine); color: var(--text-faint); background: none; border: none; padding: 0; cursor: pointer; text-decoration: underline; }
.so-says { margin: 0; padding-left: 1.1em; font-size: var(--small); color: var(--text-quiet); line-height: 1.6; }
.so-acts { display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; }
.so-gap { flex: 1; }
.so-foot { display: flex; align-items: center; gap: var(--s3); margin-top: var(--s4); }
.so-add { display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; margin-top: var(--s3); }
.so-why { font-size: var(--small); color: var(--text-quiet); line-height: 1.55; margin: 0 0 var(--s3); }
:global([data-theme="retro"] .so-anchor) { box-shadow: inset 0 0 0 var(--bw) var(--edge), var(--lift-panel); }
</style>
