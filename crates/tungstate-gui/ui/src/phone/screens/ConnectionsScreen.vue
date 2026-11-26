<!-- The Connections tab: every place Tungstate can reach (a NAS share, FTP,
     SFTP, WebDAV, S3), added and removed here and used by the other tabs.
     Tap one to open it in a Files tab of its own; "⋯" checks or removes it,
     and for FTP chooses its folder or adds another folder on the same server. Below,
     the file servers on this Wi-Fi that are not added yet: Add fills in the
     address, so only the username and password are typed. -->
<script setup lang="ts">
import { computed, onMounted, ref, shallowRef, watch } from "vue";
import { connections } from "../../engine/commands";
import type { Connection, ConnectionForm, FoundServer } from "../../engine/types";
import { offersLine } from "../lib/discovery";
import { useDiscovery } from "../state/useDiscovery";
import { kindOf } from "../../lib/kinds";
import Pixels from "../../ui/Pixels.vue";
import { useConnections } from "../../state/useConnections";
import { usePhoneNav } from "../nav";
import { useWindows } from "../state/useWindows";
import PScreen from "../ui/PScreen.vue";
import PTile from "../ui/PTile.vue";
import { connectionLook } from "../lib/look";
import PButton from "../ui/PButton.vue";
import PSheet from "../ui/PSheet.vue";
import PFolderPicker from "../ui/PFolderPicker.vue";
import PTrouble from "../ui/PTrouble.vue";
import AddConnection from "./AddConnection.vue";

const conns = useConnections();
const nav = usePhoneNav();
const w = useWindows();
const adding = ref(false);
const net = useDiscovery();
/** The found server the form starts from, or null for one added by hand. */
const from = shallowRef<FoundServer | null>(null);
/** A saved FTP connection whose server gets another folder. */
const like = shallowRef<Connection | null>(null);
function addFound(f: FoundServer) {
  from.value = f;
  like.value = null;
  adding.value = true;
}
function addByHand() {
  from.value = null;
  like.value = null;
  adding.value = true;
}
function addAnotherFolder(c: Connection) {
  about.value = null;
  from.value = null;
  like.value = c;
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
/** A check that failed, and where, for a plain sentence with Details. */
const failed = ref<{ error: string; where: string } | null>(null);
/** The FTP connection the last check found has no folder yet. */
const folderless = shallowRef<Connection | null>(null);
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
  w.openTab({ kind: "folder", path: `${c.name}:` });
  nav.go("files");
}

/** In words and a mark, never colour alone: online, unreachable, not checked. */
function status(c: Connection): { text: string; state: "ok" | "bad" | "unknown" } {
  // FTP shows its folder too: one server can hold a connection per folder.
  const tag = `${kindOf(c.scheme).tag} · ${isFtp(c) ? c.place : c.host || c.place}`;
  if (!c.last_check) return { text: `${tag} · not checked`, state: "unknown" };
  return c.last_check.ok
    ? { text: `${tag} · online`, state: "ok" }
    : { text: `${tag} · could not be reached`, state: "bad" };
}

const isFtp = (c: Connection) => c.scheme === "ftp" || c.scheme === "ftps";
/** An FTP connection saved before folders were chosen: it starts at the
 *  server's own `/`, which rarely takes files. */
const needsFolder = (c: Connection) => isFtp(c) && !c.root.trim();

/** Says how a check went: reachable, or why not in a plain sentence. */
function report(name: string) {
  const now = conns.all.value.find((x) => x.name === name);
  said.value = null;
  failed.value = null;
  folderless.value = null;
  if (now?.last_check?.ok) said.value = `${name} is reachable.`;
  else if (now && needsFolder(now)) {
    said.value = `${name} needs a folder: the top of the server doesn't take new files.`;
    folderless.value = now;
  }
  else if (now?.last_check) failed.value = { error: now.last_check.note, where: name };
}

async function checkNow() {
  const c = about.value;
  if (!c) return;
  about.value = null;
  await conns.check(c.name);
  report(c.name);
}

// ── choosing an FTP connection's folder ──────────────────────────────────
const choosing = shallowRef<Connection | null>(null);
const folder = ref("/");
const chooseTrouble = ref<string | null>(null);
// An answer about one folder says nothing about the next one opened.
watch(folder, () => (chooseTrouble.value = null));
const formOf = (c: Connection, root = ""): ConnectionForm => ({ name: c.name, scheme: c.scheme, host: c.host, port: c.port, username: c.username, root, options: c.options });
function chooseFolder(c: Connection) {
  about.value = null;
  folderless.value = null;
  said.value = null;
  folder.value = c.root.trim() || "/";
  chooseTrouble.value = null;
  choosing.value = c;
}
/** Checks the folder takes files with the saved password, then keeps it. */
async function useFolder() {
  const c = choosing.value;
  if (!c) return;
  saving.value = true;
  chooseTrouble.value = null;
  try {
    const form = formOf(c, folder.value);
    const probe = await connections.testSettings(form, null);
    if (probe.accepts_files === false) return void (chooseTrouble.value = `${folder.value} refuses files`);
    await connections.update(c.name, form);
    w.rehome(`${c.name}:`);
    choosing.value = null;
    await conns.check(c.name);
    report(c.name);
  } catch (e) {
    chooseTrouble.value = String(e);
  } finally {
    saving.value = false;
  }
}
const leaf = (path: string) => path.replace(/\/+$/, "").split("/").pop();

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
    report(c.name);
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
  like.value = null;
  await conns.load();
  const made = conns.all.value.find((c) => c.name === name);
  if (made) open(made);
}
</script>

<template>
  <AddConnection v-if="adding" :found="from" :like="like" @cancel="adding = false; from = null; like = null" @saved="added" />

  <PScreen v-else-if="choosing" title="Choose a folder" :back="choosing.name" @back="choosing = null">
    <PFolderPicker v-model="folder" :form="formOf(choosing)" :secret="null" :where="choosing.name" />
    <template #act>
      <PTrouble v-if="chooseTrouble" :error="chooseTrouble" :where="choosing.name" />
      <p v-else class="line">{{ choosing.name }} will open in this folder.</p>
      <PButton tint="var(--tint-connections)" :busy="saving" @press="useFolder">{{ saving ? "Checking…" : leaf(folder) ? `Use ${leaf(folder)}` : "Use the server's top" }}</PButton>
    </template>
  </PScreen>

  <PScreen v-else title="Connections" :tile="{ art: 'connections', tint: 'var(--tint-connections)', light: true }">
    <template #corner>
      <button class="iconbtn" aria-label="Add a connection" @click="addByHand">+</button>
    </template>
    <PTrouble v-if="conns.problem.value" :error="conns.problem.value" />
    <PTrouble v-if="trouble" :error="trouble" />
    <p v-if="said" class="line">{{ said }}</p>
    <PButton v-if="folderless" class="inline" quiet @press="chooseFolder(folderless)">Choose a folder</PButton>
    <PTrouble v-if="failed" :error="failed.error" :where="failed.where" />
    <div v-if="reachable.length" class="list">
      <div v-for="c in reachable" :key="c.name" class="row">
        <button class="open" @click="open(c)">
          <PTile v-bind="connectionLook(c.scheme)" :size="40" />
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
        <button v-if="isFtp(about)" class="option" @click="chooseFolder(about)">{{ needsFolder(about) ? "Choose folder" : "Change folder" }}</button>
        <button v-if="isFtp(about)" class="option" @click="addAnotherFolder(about)">Add another folder</button>
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
.gh { display: flex; align-items: center; justify-content: space-between; margin: 22px 0 8px; }
.gh h2 { margin: 0; font-size: 15px; font-weight: 800; }
.smallbtn { height: 36px; padding: 0 12px; font: inherit; font-size: 14px; font-weight: 800; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 3px 3px 0 var(--edge); }
.smallbtn:disabled { box-shadow: none; opacity: 0.7; }
.found .frow { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 72px; padding: 12px 14px; font: inherit; text-align: left; color: var(--text); background: var(--panel); border: 0; border-top: 2px dashed var(--edge); }
.found .frow:first-child { border-top: 0; }
.addpill { flex: none; display: grid; place-items: center; height: 44px; padding: 0 14px; font-size: 15px; font-weight: 800; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 3px 3px 0 var(--edge); }
.note { margin-top: 12px; }
.inline { margin-bottom: 14px; }
.options { display: flex; flex-direction: column; overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.option { min-height: 52px; padding: 0 16px; font: inherit; font-size: 16px; font-weight: 700; text-align: left; color: var(--text); background: none; border: 0; border-top: 2px solid var(--edge); }
.option:first-child { border-top: 0; }
.danger { color: var(--bad); }
.field { height: 52px; padding: 0 14px; font: inherit; font-size: 17px; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
</style>
