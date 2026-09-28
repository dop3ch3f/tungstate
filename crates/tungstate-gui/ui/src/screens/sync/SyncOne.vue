<!-- One sync: its folders in a row, what it does in words, and everything that
     can be done to it. Previewing is the main thing; running only ever follows
     a preview. -->
<script setup lang="ts">
import { computed, ref, shallowRef } from "vue";
import { syncs } from "../../engine/commands";
import type { PastSync, SyncSettingsForm } from "../../engine/types";
import { exactly, following, reason, way, CONFLICTS } from "../../lib/syncwords";
import { useSync } from "../../state/useSync";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import ActionBar from "../../ui/ActionBar.vue";
import Field from "../../ui/Field.vue";
import Notice from "../../ui/Notice.vue";
import Sheet from "../../ui/Sheet.vue";
import PlacePicker from "../../ui/PlacePicker.vue";
import SyncRuns from "./SyncRuns.vue";
import SyncSettings from "./SyncSettings.vue";
import SyncUndoneNotice from "./SyncUndoneNotice.vue";
import { useBusy } from "../../state/useBusy";

const s = useSync();
const sync = computed(() => s.current.value!);
const runs = computed(() => s.past.value.filter((r) => r.sync === sync.value.name));

const editing = ref<SyncSettingsForm | null>(null);
const adding = ref(false);
const forgetting = ref(false);
const forgetPath = ref("");
/** Whether the shared place picker is open over the Add sheet. */
const choosing = ref(false);
/** The folder chosen in the Add sheet, not yet added. */
const picked = ref<string | null>(null);
const calledAs = ref("");
const said = ref<string | null>(null);
const problem = ref<string | null>(null);
const doing = useBusy();

/** When it runs, short enough for the one line of settings. */
const LAUNCH_SHORT: Record<string, string> = {
  no: "Runs when you press Run",
  ask: "Asks when the app opens",
  quietly: "Runs when the app opens",
  continuous: "Kept in step while the app is open",
};
// A lookup, because the CSS checker cannot follow a computed class name.
const DOT = { hold: "so-dot-hold", ok: "so-dot-ok" } as const;
const dotOf = (member: string) => (s.statusOf(sync.value.name, member)?.state === "paused" ? DOT.hold : DOT.ok);
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
  const form = editing.value;
  if (!form) return;
  problem.value = null;
  await doing.run("save", async () => {
    try {
      s.current.value = await syncs.change(sync.value.name, form);
      editing.value = null;
      said.value = "Saved. It applies from the next run.";
      await s.load();
    } catch (e) {
      problem.value = reason(e);
    }
  });
}

function openAdding() {
  picked.value = null;
  calledAs.value = "";
  problem.value = null;
  adding.value = true;
}

/** Add a member. Under the "add" key, so a second pick while the first is
 *  being added is not added twice. */
async function add(end: string) {
  problem.value = null;
  try {
    s.current.value = await syncs.addMember(sync.value.name, end, calledAs.value.trim() || null);
    adding.value = false;
    calledAs.value = "";
    said.value = "Added. The next run fills it.";
    await s.load();
  } catch (e) {
    problem.value = reason(e);
  }
}

const addEnd = (end: string) => doing.run("add", () => add(end));

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
  await doing.run(`take:${member}`, async () => {
    try {
      s.current.value = await syncs.removeMember(sync.value.name, member);
      await s.load();
    } catch (e) {
      problem.value = reason(e);
    }
  });
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
  await doing.run("remove", async () => {
    try {
      await s.remove(sync.value.name);
      editing.value = null;
    } catch (e) {
      problem.value = reason(e);
    }
  });
}

async function forget() {
  const path = forgetPath.value.trim();
  if (!path) return;
  forgetting.value = false;
  forgetPath.value = "";
  await s.forget([path]);
}

async function putBack(run: PastSync) {
  if (s.busy.value != null) return;
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
    <Notice tone="hold" v-if="s.isHeld(sync.name)">
      Stopped keeping in step: its next run would remove files, so it waits for you.
      <Button look="link" @click="s.look()">Look at it</Button>
    </Notice>

    <p class="so-says">
      {{ way(sync) }} · {{ exactly(sync) }} · {{ conflictWords }} · {{ LAUNCH_SHORT[sync.launch] }}
      <Button look="link" @click="edit()">Change</Button>
    </p>

    <ol class="so-places">
      <li v-for="member in sync.members" :key="member.name" class="so-place">
        <span class="so-who">
          <b>{{ member.name }}</b>
          <span class="so-role" v-if="role(member.name)">{{ role(member.name) }}</span>
        </span>
        <span class="path so-at" :title="member.at">{{ member.at }}</span>
        <span class="so-state" v-if="s.statusOf(sync.name, member.name)" :title="s.statusOf(sync.name, member.name)?.why ?? undefined">
          <span class="so-dot" :class="dotOf(member.name)"></span>
          <span :class="{ 'so-paused': s.statusOf(sync.name, member.name)?.state === 'paused' }">{{ following(s.statusOf(sync.name, member.name)!) }}</span>
        </span>
        <span v-else></span>
        <button
          class="so-drop"
          v-if="sync.members.length > 2 && !role(member.name)"
          @click="removeMember(member.name)"
          :disabled="doing.busy(`take:${member.name}`)"
          :aria-busy="doing.busy(`take:${member.name}`) || undefined"
          :title="`Take ${member.name} out`"
        >Take out</button>
      </li>
    </ol>

    <div class="so-acts">
      <Button look="primary" @click="s.look()">Preview a run</Button>
      <Button @click="openAdding()">Add a folder…</Button>
      <Button @click="forgetting = true">Forget a file…</Button>
    </div>

    <Notice v-if="said">{{ said }}</Notice>
    <SyncUndoneNotice />
    <Notice tone="bad" v-if="problem || s.problem.value">{{ problem ?? s.problem.value }}</Notice>

    <SyncRuns single :title="`Runs of ${sync.name}`" :runs="runs" :busy="s.busy.value" @put-back="putBack" />

    <Sheet v-if="editing" wide of="sync" :title="`Settings for ${sync.name}`" @dismiss="editing = null">
      <SyncSettings v-model="editing" />
      <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
      <ActionBar pinned>
        <!-- Rarely wanted and never undone by accident: here rather than
             beside Preview on the sync's page. -->
        <template #say><Button look="danger" :busy="doing.busy('remove')" @click="removeSync()">Remove this sync</Button></template>
        <Button @click="editing = null">Cancel</Button>
        <Button look="primary" :busy="doing.busy('save')" @click="saveSettings()">Save</Button>
      </ActionBar>
    </Sheet>

    <Sheet v-if="adding" of="sync" :title="`Add a folder to ${sync.name}`" @dismiss="adding = false">
      <Field label="Folder" note="On this Mac, a drive, or a place on a connection.">
        <div class="so-pick">
          <span class="path so-picked" v-if="picked">{{ picked }}</span>
          <Button @click="choosing = true">{{ picked ? "Change…" : "Choose…" }}</Button>
        </div>
      </Field>
      <Field label="What to call it" note="How this folder is named in the sync. Empty uses the folder's own name.">
        <input v-model="calledAs" :placeholder="picked ? picked.split(/[/:]/).filter(Boolean).pop() : ''" />
      </Field>
      <PlacePicker
        v-if="choosing"
        of="sync"
        :title="`A folder for ${sync.name}`"
        choose="Use this folder"
        @dismiss="choosing = false"
        @chosen="(end) => { choosing = false; picked = end; }"
      />
      <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
      <ActionBar pinned>
        <Button @click="adding = false">Cancel</Button>
        <Button look="primary" :disabled="!picked" :busy="doing.busy('add')" @click="picked && addEnd(picked)">Add</Button>
      </ActionBar>
    </Sheet>

    <Sheet v-if="forgetting" of="sync" title="Forget a file" @dismiss="forgetting = false">
      <p class="so-why">
        Takes a file or folder off every folder in this sync for good. You see
        what would go before anything does.
      </p>
      <Field label="File or folder" note="As the preview lists it, e.g. exports/old.mp4 or a whole folder.">
        <input v-model="forgetPath" placeholder="exports/old.mp4" @keydown.enter="forget()" />
      </Field>
      <ActionBar pinned>
        <Button @click="forgetting = false">Cancel</Button>
        <Button look="primary" :disabled="!forgetPath.trim()" @click="forget()">Show what would go</Button>
      </ActionBar>
    </Sheet>
  </div>
</template>

<style scoped>
.so { display: flex; flex-direction: column; gap: var(--s4); }
.so-places { list-style: none; margin: 0; padding: 0; border-top: 1px solid var(--rule); }
/* Rows on fixed columns, as Connections has, so four folders read as a list
   rather than a wrap of tiles with one left over. */
.so-place {
  display: grid;
  grid-template-columns: 170px minmax(0, 1fr) 260px 64px;
  gap: var(--s3);
  align-items: baseline;
  padding: var(--s3) 0;
  border-bottom: 1px solid var(--rule);
  font-size: var(--small);
}
.so-who { display: flex; align-items: baseline; gap: var(--s2); min-width: 0; }
.so-at { font-size: var(--fine); color: var(--text-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.so-role { font-size: var(--fine); font-weight: 700; color: var(--control-ink); background: var(--control); border-radius: var(--radius); padding: 0 6px; }
.so-state { display: flex; align-items: baseline; gap: var(--s2); font-size: var(--fine); color: var(--text-quiet); min-width: 0; }
.so-dot { width: 7px; height: 7px; border-radius: 50%; flex: none; transform: translateY(-1px); }
.so-dot-ok { background: var(--ok); }
.so-dot-hold { background: var(--hold); }
.so-paused { color: var(--hold); }
.so-drop[aria-busy="true"] { cursor: progress; opacity: 0.6; }
.so-drop { justify-self: end; font: inherit; font-size: var(--fine); color: var(--text-faint); background: none; border: none; padding: 0; cursor: pointer; text-decoration: underline; }
.so-says { margin: calc(-1 * var(--s2)) 0 0; font-size: var(--small); color: var(--text-quiet); display: flex; align-items: baseline; gap: var(--s2); flex-wrap: wrap; }
.so-acts { display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; }
.so-pick { display: flex; align-items: center; gap: var(--s3); min-width: 0; }
.so-picked { font-size: var(--small); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.so-why { font-size: var(--small); color: var(--text-quiet); line-height: 1.55; margin: 0 0 var(--s3); }
</style>
