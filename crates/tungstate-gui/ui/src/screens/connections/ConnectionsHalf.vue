<!-- Every place Tungstate can reach besides this Mac, set up once and used by
     every section. Each row says where it points and how its last check went,
     because a place that lists and refuses files is the failure that matters. -->
<script setup lang="ts">
import { onMounted, ref } from "vue";
import { connections } from "../../engine/commands";
import type { Connection } from "../../engine/types";
import { useConnections } from "../../state/useConnections";
import { useBusy } from "../../state/useBusy";
import { useNav } from "../../nav";
import { kindOf } from "../../lib/kinds";
import { plural, when } from "../../lib/format";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import Steps from "../../ui/Steps.vue";
import Sheet from "../../ui/Sheet.vue";
import Field from "../../ui/Field.vue";
import Tile from "../../ui/Tile.vue";
import ConnectionForm from "./ConnectionForm.vue";

const c = useConnections();
const nav = useNav();
const form = ref<{ editing: Connection | null } | null>(null);
const changing = ref<Connection | null>(null);
const newSecret = ref("");
const said = ref<string | null>(null);
const doing = useBusy();

/** The warning in a few words on the row; the whole of it is its tooltip and
 *  in the form, where the choice that changes it is made. */
const CLEAR: Record<string, string> = {
  smb: "Encrypted only if the server asks",
  ftp: "Password and files sent unencrypted",
  s3: "Plain http: files are not encrypted",
};

onMounted(() => void c.load());

const STEPS = [
  { art: "connections", title: "Choose what kind", line: "A NAS share, an FTP server, an S3 bucket, or a drive this Mac has mounted." },
  { art: "in-step", title: "Check it works", line: "Before it is saved, so a wrong password is found now and not halfway through a transfer." },
  { art: "drain", title: "Use it anywhere", line: "Transfer, Sync and Duplicates all offer it wherever they ask for a place." },
] as const;

async function saved(name: string) {
  form.value = null;
  await c.load();
  // Checked again once saved: proving it works is why anyone opened the form.
  await c.check(name);
}

function browse(name: string) {
  c.browseTo.value = `${name}:`;
  nav.go("drain");
}

async function savePassword() {
  const target = changing.value;
  if (!target) return;
  await doing.run("password", async () => {
    await connections.setPassword(target.name, newSecret.value);
    changing.value = null;
    newSecret.value = "";
  });
  if (!changing.value) await c.check(target.name);
}

/** Say what still uses it before anything is asked, and what Delete will
 *  mean: gone, or put away because History still names it. */
async function remove(target: Connection) {
  await doing.run(`remove:${target.name}`, async () => {
    const uses = await connections.uses(target.name);
    const blocking = [
      ...uses.pairs.map((p) => `Saved pair: ${p}`),
      ...uses.syncs.map((s) => `Sync: ${s}`),
      ...uses.unfinished.map((u) => `Stopped part-way: ${u}`),
    ];
    if (blocking.length) {
      await ask({
        title: `${target.name} is still in use`,
        why: "Remove these first, or finish or clear the transfers that stopped part-way. Then it can go.",
        detail: blocking,
        choices: [{ id: "ok", label: "OK", look: "primary" }],
      });
      return;
    }
    const answer = await ask({
      title: `Delete ${target.name}?`,
      why: uses.history
        ? `History names it in ${plural(uses.history, "past transfer")}, so it is put away rather than deleted: those keep saying where their files went. Its name and its password are freed.`
        : "It has never been used, so it goes for good, with its password.",
      choices: [
        { id: "no", label: "Keep it" },
        { id: "yes", label: "Delete", look: "danger" },
      ],
    });
    if (answer.id !== "yes") return;
    const how = await connections.remove(target.name);
    said.value = how === "retired" ? `Put ${target.name} away. History still shows where its files went.` : `Deleted ${target.name}.`;
    await c.load();
  });
}
</script>

<template>
  <div class="cn">
    <div class="cn-inner">
      <div class="cn-head">
        <Tile of="connections" :size="26" /><h1>Connections</h1>
      </div>
      <p class="cn-lede">
        Every place besides this Mac: a NAS, a server, a bucket. Set each one up once, and every
        section can use it.
      </p>
      <div class="cn-bar">
        <Button look="primary" @click="form = { editing: null }">Add a connection</Button>
        <Button v-if="c.all.value.length" :busy="c.checkingAll()" @click="c.checkAll()">Check all</Button>
      </div>

      <Notice tone="bad" v-if="c.problem.value">{{ c.problem.value }}</Notice>
      <Notice tone="bad" v-if="doing.problem.value">{{ doing.problem.value }}</Notice>
      <Notice v-if="said">{{ said }}</Notice>

      <Steps v-if="c.loaded.value && !c.all.value.length" :steps="[...STEPS]" />

      <ul class="cn-list" v-if="c.all.value.length">
        <li class="cn-row" v-for="item in c.all.value" :key="item.name">
          <div class="cn-who">
            <span class="cn-tag">{{ kindOf(item.scheme).tag }}</span>
            <b class="cn-name">{{ item.name }}</b>
            <span class="path cn-place">{{ item.place }}</span>
          </div>
          <div class="cn-state">
            <span v-if="c.checking(item.name)" class="cn-quiet">Checking…</span>
            <template v-else-if="item.last_check">
              <span class="cn-dot" :class="item.last_check.ok ? 'cn-ok' : 'cn-bad'"></span>
              <span :class="item.last_check.ok ? 'cn-quiet' : 'cn-fail'">
                {{ item.last_check.ok ? item.last_check.note : `Did not work: ${item.last_check.note}` }}
              </span>
              <span class="cn-when">{{ when(item.last_check.at) }}</span>
            </template>
            <span v-else class="cn-quiet">Not checked yet</span>
            <span class="cn-warn" v-if="item.clear" :title="item.clear">{{ CLEAR[item.scheme] ?? "Not encrypted" }}</span>
            <span class="cn-warn" v-if="item.rootless">{{ item.rootless }}</span>
          </div>
          <div class="cn-do">
            <Button :busy="c.checking(item.name)" @click="c.check(item.name)">Check</Button>
            <Button @click="browse(item.name)">Browse</Button>
            <Button look="link" @click="form = { editing: item }">Edit</Button>
            <Button look="link" v-if="item.scheme !== 'fs'" @click="changing = item; newSecret = ''">
              {{ item.scheme === "s3" ? "Secret key" : "Password" }}
            </Button>
            <Button look="link" :busy="doing.busy(`remove:${item.name}`)" @click="remove(item)">Delete</Button>
          </div>
        </li>
      </ul>
    </div>

    <ConnectionForm v-if="form" :editing="form.editing" @dismiss="form = null" @saved="saved" />

    <Sheet
      v-if="changing"
      of="connections"
      :title="`${changing.scheme === 's3' ? 'Secret key' : 'Password'} for ${changing.name}`"
      @dismiss="changing = null; newSecret = ''"
    >
      <Field
        :label="changing.scheme === 's3' ? 'Secret key' : 'Password'"
        note="Replaces the one in this machine's keychain. It is checked straight after."
      >
        <input type="password" v-model="newSecret" autocomplete="off" />
      </Field>
      <div class="cn-foot">
        <Button @click="changing = null; newSecret = ''">Cancel</Button>
        <Button look="primary" :disabled="!newSecret" :busy="doing.busy('password')" @click="savePassword()">Save it</Button>
      </div>
    </Sheet>
  </div>
</template>

<style scoped>
.cn { position: absolute; inset: 0; overflow-y: auto; scrollbar-gutter: stable; }
.cn-inner { padding: var(--win-pad); display: flex; flex-direction: column; gap: var(--s4); min-height: 100%; container-type: inline-size; }
.cn-head { display: flex; align-items: center; gap: var(--s3); }
.cn-head h1 { font-size: var(--display); font-weight: 700; margin: 0; letter-spacing: -0.01em; }
.cn-lede { font-size: var(--small); color: var(--text-quiet); margin: 0; max-width: 62ch; line-height: 1.5; }
.cn-bar { display: flex; gap: var(--s2); }

.cn-list { list-style: none; margin: 0; padding: 0; }
.cn-row {
  display: grid;
  grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr) auto;
  gap: var(--s4);
  align-items: start;
  padding: var(--s3) 0;
  border-bottom: var(--bw) solid var(--rule);
}
@container (max-width: 720px) {
  .cn-row { grid-template-columns: minmax(0, 1fr); gap: var(--s2); }
}
.cn-who { display: grid; grid-template-columns: auto minmax(0, 1fr); column-gap: var(--s2); row-gap: 2px; align-items: baseline; min-width: 0; }
.cn-tag {
  font-size: var(--fine);
  font-weight: 600;
  color: var(--text-quiet);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius);
  padding: 0 5px;
}
.cn-name { font-size: var(--body); }
.cn-place { grid-column: 2; font-size: var(--fine); color: var(--text-faint); }
.cn-state { display: flex; flex-wrap: wrap; align-items: baseline; gap: 2px var(--s2); font-size: var(--fine); }
.cn-dot { width: 7px; height: 7px; border-radius: 50%; align-self: center; }
.cn-ok { background: var(--ok); }
.cn-bad { background: var(--bad); }
.cn-quiet { color: var(--text-quiet); }
.cn-fail { color: var(--bad); }
.cn-when { color: var(--text-faint); }
.cn-warn { flex-basis: 100%; color: var(--hold); line-height: 1.45; }
.cn-do { display: flex; gap: var(--s2); align-items: center; flex-wrap: wrap; justify-content: flex-end; }
.cn-foot { display: flex; justify-content: flex-end; gap: var(--s2); margin-top: var(--s5); }
</style>
