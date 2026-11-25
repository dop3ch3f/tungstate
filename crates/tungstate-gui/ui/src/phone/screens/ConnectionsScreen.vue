<!-- The Connections tab: every place Tungstate can reach (a NAS share, FTP,
     SFTP, WebDAV, S3), added and removed here and used by the other tabs.
     Tap one to open it in a Files window; "⋯" checks or removes it. Below,
     the file servers on this Wi-Fi that are not added yet: Add fills in the
     address, so only the username and password are typed. -->
<script setup lang="ts">
import { computed, onMounted, ref, shallowRef, watch } from "vue";
import { connections } from "../../engine/commands";
import type { Connection, FoundServer } from "../../engine/types";
import { offersLine } from "../lib/discovery";
import { useDiscovery } from "../state/useDiscovery";
import { kindOf } from "../../lib/kinds";
import Pixels from "../../ui/Pixels.vue";
import { useConnections } from "../../state/useConnections";
import { usePhoneNav } from "../nav";
import { useWindows } from "../state/useWindows";
import PScreen from "../ui/PScreen.vue";
import PTile from "../ui/PTile.vue";
import PButton from "../ui/PButton.vue";
import PSheet from "../ui/PSheet.vue";
import AddConnection from "./AddConnection.vue";

const conns = useConnections();
const nav = usePhoneNav();
const w = useWindows();
const adding = ref(false);
const net = useDiscovery();
/** The found server the form starts from, or null for one added by hand. */
const from = shallowRef<FoundServer | null>(null);
function addFound(f: FoundServer) {
  from.value = f;
  adding.value = true;
}
function addByHand() {
  from.value = null;
  adding.value = true;
}
// Add on Files home sends the server here to be added.
watch(net.adding, (f) => {
  if (!f) return;
  addFound(f);
  net.adding.value = null;
}, { immediate: true });
/** The connection whose "⋯" is open, and whether removal is being asked. */
const about = ref<Connection | null>(null);
const confirming = ref(false);
const trouble = ref<string | null>(null);
/** A new password being typed for one connection. */
const password = ref<string | null>(null);
const saving = ref(false);
const said = ref<string | null>(null);

// A folder the device mounts means nothing on a phone.
const reachable = computed(() => conns.all.value.filter((c) => c.scheme !== "fs"));
onMounted(() => {
  void conns.load();
  if (!net.looked.value) void net.look();
});

function open(c: Connection) {
  w.go({ kind: "folder", path: `${c.name}:` });
  nav.go("files");
}

/** In words and a mark, never colour alone: online, unreachable, not checked. */
function status(c: Connection): { text: string; state: "ok" | "bad" | "unknown" } {
  const tag = `${kindOf(c.scheme).tag} · ${c.host || c.place}`;
  if (!c.last_check) return { text: `${tag} · not checked`, state: "unknown" };
  return c.last_check.ok
    ? { text: `${tag} · online`, state: "ok" }
    : { text: `${tag} · could not be reached`, state: "bad" };
}

async function checkNow() {
  const c = about.value;
  if (!c) return;
  about.value = null;
  await conns.check(c.name);
  const now = conns.all.value.find((x) => x.name === c.name);
  said.value = now?.last_check?.ok ? `${c.name} is reachable.` : now?.last_check?.note ?? null;
}

/** The saved password is kept in the phone's keystore, which a reinstall
 *  clears; this is how a person puts it back without adding it again. */
async function savePassword() {
  const c = about.value;
  if (!c || !password.value) return;
  saving.value = true;
  try {
    await connections.setPassword(c.name, password.value);
    password.value = null;
    about.value = null;
    await conns.check(c.name);
    const now = conns.all.value.find((x) => x.name === c.name);
    said.value = now?.last_check?.ok ? `${c.name} is reachable.` : now?.last_check?.note ?? null;
  } catch (e) {
    trouble.value = String(e);
  } finally {
    saving.value = false;
  }
}

async function remove() {
  const c = about.value;
  if (!c) return;
  about.value = null;
  confirming.value = false;
  try {
    const how = await connections.remove(c.name);
    await conns.load();
    // History still names it, so it is kept under its name but no longer offered.
    said.value = how === "retired" ? `${c.name} was removed. History still mentions it.` : `${c.name} was removed.`;
  } catch (e) {
    trouble.value = String(e);
  }
}

async function added(name: string) {
  adding.value = false;
  from.value = null;
  await conns.load();
  const made = conns.all.value.find((c) => c.name === name);
  if (made) open(made);
}
</script>

<template>
  <AddConnection v-if="adding" :found="from" @cancel="adding = false; from = null" @saved="added" />

  <PScreen v-else title="Connections">
    <template #corner>
      <button class="iconbtn" aria-label="Add a connection" @click="addByHand">+</button>
    </template>
    <p v-if="conns.problem.value" class="line bad">{{ conns.problem.value }}</p>
    <p v-if="trouble" class="line bad">{{ trouble }}</p>
    <p v-if="said" class="line">{{ said }}</p>
    <div v-if="reachable.length" class="list">
      <div v-for="c in reachable" :key="c.name" class="row">
        <button class="open" @click="open(c)">
          <PTile art="nas" tint="var(--tint-connections)" light :size="40" />
          <span class="text">
            <b>{{ c.name }}</b>
            <span class="quiet"><i class="dot" :class="status(c).state" aria-hidden="true"></i>{{ status(c).text }}</span>
          </span>
        </button>
        <button class="dots" :aria-label="`More for ${c.name}`" @click="about = c"><Pixels of="more" :size="20" /></button>
      </div>
    </div>
    <p v-else-if="conns.loaded.value" class="line">Nothing added yet. Every other tab sends to and reads from these.</p>

    <div class="gh">
      <h2>On your network</h2>
      <button class="smallbtn" :disabled="net.looking.value" @click="net.look()">{{ net.looking.value ? "Looking…" : "Look again" }}</button>
    </div>
    <div v-if="net.fresh.value.length" class="list found">
      <button v-for="f in net.fresh.value" :key="f.address" class="frow" @click="addFound(f)">
        <PTile art="nas" :tint="f.name ? 'var(--tint-connections)' : '#a5a79d'" light :size="40" />
        <span class="text"><b>{{ f.name ?? f.address }}</b><span class="quiet">{{ offersLine(f) }} · {{ f.name ? f.address : "gave no name" }}</span></span>
        <span class="addpill">Add</span>
      </button>
    </div>
    <p v-if="net.fresh.value.length" class="line note">Add fills in the address. You only type the username and password.</p>
    <p v-else-if="net.looked.value && !net.looking.value" class="line note">No file servers answered on this Wi-Fi. Check the phone is on the same network as your NAS, or add one by hand.</p>

    <PSheet v-if="about && !confirming && password === null" :title="about.name" :says="about.place" @close="about = null">
      <div class="options">
        <button class="option" @click="open(about)">Open</button>
        <button class="option" @click="checkNow">Check now</button>
        <button class="option" @click="password = ''">Change password</button>
        <button class="option danger" @click="confirming = true">Remove {{ about.name }}</button>
      </div>
    </PSheet>
    <PSheet v-if="about && password !== null" :title="`Password for ${about.name}`" says="Kept in this phone's keystore. Tungstate checks it right away." @close="password = null">
      <input :value="password" class="field" @input="password = ($event.target as HTMLInputElement).value" type="password" autocomplete="off" aria-label="Password" @keydown.enter="savePassword" />
      <PButton tint="var(--tint-connections)" :busy="saving" :disabled="!password" @press="savePassword">Save and check</PButton>
    </PSheet>
    <PSheet v-if="about && confirming" :title="`Remove ${about.name}?`" says="Its saved password is forgotten. Files on it are not touched." @close="confirming = false">
      <PButton tint="var(--bad)" @press="remove">Remove</PButton>
      <PButton quiet @press="confirming = false">Keep it</PButton>
    </PSheet>

    <template #act>
      <PButton quiet @press="addByHand">Add one by hand</PButton>
    </template>
  </PScreen>
</template>

<style scoped>
.list { overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.row { display: flex; align-items: center; border-top: 2px solid var(--edge); }
.row:first-child { border-top: 0; }
.open { flex: 1; min-width: 0; display: flex; align-items: center; gap: 12px; min-height: 72px; padding: 12px 6px 12px 14px; font: inherit; text-align: left; color: var(--text); background: none; border: 0; }
.open:active { background: var(--surface-hover); }
.text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
.text b { font-size: 16px; }
.quiet { font-size: 14px; color: var(--text-quiet); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.dot { display: inline-block; width: 10px; height: 10px; margin-right: 6px; vertical-align: -1px; background: #a5a79d; border: 2px solid var(--edge); border-radius: 2px; }
.dot.ok { background: var(--tint-dupes); }
.dot.bad { background: var(--bad); }
.dots { flex: none; width: 52px; height: 72px; display: grid; place-items: center; color: var(--text); background: none; border: 0; }
.iconbtn { flex: none; width: 44px; height: 44px; font: inherit; font-size: 24px; font-weight: 800; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 3px 3px 0 var(--edge); }
.line { margin: 4px 2px 14px; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.bad { color: var(--bad); font-weight: 700; }
.gh { display: flex; align-items: center; justify-content: space-between; margin: 22px 0 8px; }
.gh h2 { margin: 0; font-size: 15px; font-weight: 800; }
.smallbtn { height: 36px; padding: 0 12px; font: inherit; font-size: 14px; font-weight: 800; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 3px 3px 0 var(--edge); }
.smallbtn:disabled { box-shadow: none; opacity: 0.7; }
.found .frow { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 72px; padding: 12px 14px; font: inherit; text-align: left; color: var(--text); background: var(--panel); border: 0; border-top: 2px dashed var(--edge); }
.found .frow:first-child { border-top: 0; }
.addpill { flex: none; display: grid; place-items: center; height: 44px; padding: 0 14px; font-size: 15px; font-weight: 800; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 3px 3px 0 var(--edge); }
.note { margin-top: 12px; }
.options { display: flex; flex-direction: column; overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.option { min-height: 52px; padding: 0 16px; font: inherit; font-size: 16px; font-weight: 700; text-align: left; color: var(--text); background: none; border: 0; border-top: 2px solid var(--edge); }
.option:first-child { border-top: 0; }
.danger { color: var(--bad); }
.field { height: 52px; padding: 0 14px; font: inherit; font-size: 17px; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
</style>
