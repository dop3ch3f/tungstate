<!-- The Connections tab: every place Tungstate can reach (a NAS share, FTP,
     SFTP, WebDAV, S3), added and removed here and used by the other tabs.
     Tap one to browse it a folder at a time; "⋯" checks or removes it.

     Folders show no size: the listing knows a file's size but a folder's
     would mean walking everything inside it over the network. -->
<script setup lang="ts">
import { computed, onMounted, ref, shallowRef } from "vue";
import { connections, transfers } from "../../engine/commands";
import type { Connection, Entry, Listing } from "../../engine/types";
import { ago, bytes, kind } from "../../lib/format";
import { kindOf } from "../../lib/kinds";
import type { Art } from "../../lib/pixels";
import { useConnections } from "../../state/useConnections";
import PScreen from "../ui/PScreen.vue";
import PRow from "../ui/PRow.vue";
import PTile from "../ui/PTile.vue";
import PButton from "../ui/PButton.vue";
import AddConnection from "./AddConnection.vue";

const conns = useConnections();
/** The open folder; none means the list of connections. */
const listing = shallowRef<Listing | null>(null);
/** The connection the open folder belongs to. */
const within = ref<Connection | null>(null);
const adding = ref(false);
const menu = ref(false);
/** Asking before a removal, which forgets the saved password. */
const confirming = ref(false);
const loading = ref(false);
const trouble = ref<string | null>(null);
const said = ref<string | null>(null);

// A folder the device mounts means nothing on a phone.
const reachable = computed(() => conns.all.value.filter((c) => c.scheme !== "fs"));

async function open(path: string) {
  loading.value = true;
  trouble.value = null;
  try {
    listing.value = await transfers.browse(path);
  } catch (e) {
    trouble.value = String(e);
  } finally {
    loading.value = false;
  }
}

function openConnection(c: Connection) {
  within.value = c;
  listing.value = { path: `${c.name}:`, parent: null, entries: [] };
  said.value = null;
  void open(`${c.name}:`);
}

onMounted(() => void conns.load());

const tail = (path: string) => path.replace(/^[^:]+:/, "").replace(/\/+$/, "").split("/").pop() ?? "";
const title = computed(() => (listing.value ? tail(listing.value.path) || within.value?.name || "" : "Connections"));
/** The back link names where it goes: the folder above, or the list. */
const back = computed(() => {
  const parent = listing.value?.parent;
  return parent ? tail(parent) || within.value?.name : "Connections";
});

function goBack() {
  menu.value = false;
  const parent = listing.value?.parent;
  if (parent) void open(parent);
  else listing.value = null;
}

// Folders first, then by name, as a phone's Files app lists them.
const entries = computed(() =>
  [...(listing.value?.entries ?? [])].sort((a, b) => Number(b.is_dir) - Number(a.is_dir) || a.name.localeCompare(b.name)),
);

const KIND_TINT: Record<string, string> = {
  Folder: "var(--raw-kind-folder)",
  Video: "var(--raw-kind-video)",
  Image: "var(--raw-kind-image)",
  "Raw image": "var(--raw-kind-image)",
  Audio: "var(--raw-kind-audio)",
  Archive: "var(--raw-kind-archive)",
  Document: "var(--raw-kind-doc)",
  Sheet: "var(--raw-kind-doc)",
};
function art(e: Entry): Art {
  if (e.is_dir) return "folder";
  const k = kind(e);
  return k === "Video" || k === "Image" || k === "Raw image" ? "photo" : "file";
}
const tint = (e: Entry) => KIND_TINT[kind(e)] ?? "#a5a79d";

function status(c: Connection): string {
  const tag = kindOf(c.scheme).tag;
  if (!c.last_check) return `${tag} · not checked yet`;
  return c.last_check.ok ? `${tag} · reached ${ago(c.last_check.at)}` : `${tag} · couldn't reach it`;
}

async function checkNow() {
  const c = within.value;
  if (!c) return;
  menu.value = false;
  await conns.check(c.name);
  const now = conns.all.value.find((x) => x.name === c.name);
  said.value = now?.last_check?.ok ? `${c.name} is reachable.` : now?.last_check?.note ?? null;
}

async function remove() {
  const c = within.value;
  if (!c) return;
  menu.value = false;
  try {
    const how = await connections.remove(c.name);
    listing.value = null;
    within.value = null;
    await conns.load();
    // History still names it, so it is kept under its name but no longer offered.
    if (how === "retired") said.value = `${c.name} was removed. History still mentions it.`;
  } catch (e) {
    trouble.value = String(e);
  }
}

async function added(name: string) {
  adding.value = false;
  await conns.load();
  const made = conns.all.value.find((c) => c.name === name);
  if (made) openConnection(made);
}
</script>

<template>
  <AddConnection v-if="adding" @cancel="adding = false" @saved="added" />

  <PScreen v-else-if="listing" :title="title" :back="back" @back="goBack">
    <template #corner>
      <button v-if="!listing.parent" class="more" aria-label="More" @click="menu = !menu">⋯</button>
    </template>
    <div v-if="menu" class="menu">
      <button class="item" @click="checkNow">Check now</button>
      <button class="item danger" @click="menu = false; confirming = true">Remove {{ within?.name }}</button>
    </div>
    <div v-if="confirming" class="confirm">
      <b>Remove {{ within?.name }}?</b>
      <p>Its saved password is forgotten. Files on it are not touched.</p>
      <div class="pair">
        <PButton tint="var(--bad)" @press="confirming = false; remove()">Remove</PButton>
        <PButton quiet @press="confirming = false">Keep it</PButton>
      </div>
    </div>
    <p v-if="said" class="line">{{ said }}</p>
    <p v-if="trouble" class="line bad">{{ trouble }}</p>
    <p v-else-if="loading" class="line">Opening…</p>
    <p v-else-if="!entries.length" class="line">This folder is empty.</p>
    <div v-if="entries.length" class="list" :class="{ dim: loading }">
      <PRow
        v-for="e in entries"
        :key="e.path"
        :title="e.name"
        :sub="kind(e)"
        :end="e.is_dir ? undefined : bytes(e.size)"
        :chevron="e.is_dir"
        @press="open(e.path)"
      >
        <template #lead><PTile :art="art(e)" :tint="tint(e)" /></template>
      </PRow>
    </div>
  </PScreen>

  <PScreen v-else title="Connections">
    <p v-if="conns.problem.value" class="line bad">{{ conns.problem.value }}</p>
    <p v-if="said" class="line">{{ said }}</p>
    <div v-if="reachable.length" class="list">
      <PRow v-for="c in reachable" :key="c.name" :title="c.name" :sub="status(c)" chevron @press="openConnection(c)">
        <template #lead><PTile art="connections" tint="var(--tint-connections)" light /></template>
      </PRow>
    </div>
    <div v-else-if="conns.loaded.value" class="empty">
      <PTile art="connections" tint="var(--tint-connections)" light :size="56" />
      <p class="does">Connect your NAS, an FTP or SFTP server, WebDAV or S3. Every other tab sends to and reads from these.</p>
    </div>
    <template #act>
      <PButton :quiet="reachable.length > 0" tint="var(--tint-connections)" @press="adding = true">Add a connection</PButton>
    </template>
  </PScreen>
</template>

<style scoped>
.list { background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; overflow: hidden; transition: opacity 120ms; }
.dim { opacity: 0.5; }
.line { margin: 4px 2px 14px; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.bad { color: var(--bad); font-weight: 700; }
.empty { display: flex; flex-direction: column; gap: 14px; padding: 18px; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.does { margin: 0; font-size: 17px; line-height: 1.4; font-weight: 600; }
.more { flex: none; width: 44px; height: 44px; font: inherit; font-size: 20px; font-weight: 900; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 3px 3px 0 var(--edge); }
.menu { display: flex; flex-direction: column; margin-bottom: 14px; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 4px 4px 0 var(--edge); overflow: hidden; }
.item { min-height: 52px; padding: 0 16px; font: inherit; font-size: 16px; font-weight: 700; text-align: left; color: var(--text); background: none; border: 0; border-top: 2px solid var(--edge); }
.item:first-child { border-top: 0; }
.danger { color: var(--bad); }
.confirm { display: flex; flex-direction: column; gap: 10px; margin-bottom: 14px; padding: 14px 16px; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.confirm b { font-size: 17px; }
.confirm p { margin: 0; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.pair { display: flex; gap: 12px; }
.pair > * { flex: 1; }
</style>
