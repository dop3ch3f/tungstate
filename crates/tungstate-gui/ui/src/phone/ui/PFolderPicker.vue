<!-- Choosing a folder on a server before it is saved as a connection: tap a
     folder to go in, Up to come back out. The folder shown is the one chosen;
     the screen's own button says what to do with it. Used for FTP, whose
     server-wide `/` rarely takes files. -->
<script setup lang="ts">
import { ref, shallowRef, watch } from "vue";
import { connections } from "../../engine/commands";
import type { ConnectionForm } from "../../engine/types";
import Pixels from "../../ui/Pixels.vue";
import PLoadBar from "./PLoadBar.vue";
import PTrouble from "./PTrouble.vue";

const props = defineProps<{
  form: ConnectionForm;
  /** The typed password, or null for a saved connection's own. */
  secret: string | null;
  /** What the server is called, for errors. */
  where: string;
  /** The listing of the starting folder, when the caller already has it. */
  first?: string[] | null;
}>();
/** Where the picker is: the folder that would be chosen. */
const path = defineModel<string>({ required: true });

const folders = shallowRef<string[]>(props.first ?? []);
const loading = ref(false);
const trouble = ref<unknown>(null);

const inside = (folder: string, name: string) => `${folder.replace(/\/+$/, "")}/${name}`;
const up = (folder: string) => folder.replace(/\/+$/, "").replace(/\/[^/]*$/, "") || "/";
/** The last two folders of a deep path, `…/me/Photos`, so it fits a line. */
function shown(folder: string): string {
  const parts = folder.split("/").filter(Boolean);
  return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : `/${parts.join("/")}`;
}

// Taps through folders can answer out of order; only the latest is drawn.
let asked = 0;
async function load(at: string) {
  const mine = ++asked;
  loading.value = true;
  trouble.value = null;
  try {
    const got = await connections.folders(props.form, props.secret, at);
    if (mine === asked) folders.value = got;
  } catch (e) {
    if (mine === asked) {
      folders.value = [];
      trouble.value = e;
    }
  } finally {
    if (mine === asked) loading.value = false;
  }
}
watch(path, (at) => void load(at), { immediate: !props.first });
</script>

<template>
  <div class="picker">
    <div class="where">
      <button class="up" :disabled="path === '/'" aria-label="Up one folder" @click="path = up(path)">‹ Up</button>
      <span class="path">{{ path === "/" ? "/ (the server's top)" : shown(path) }}</span>
    </div>
    <div class="list">
      <PLoadBar v-if="loading && !folders.length" class="wait" label="Listing folders" />
      <button v-for="f in folders" :key="f" class="row" @click="path = inside(path, f)">
        <Pixels of="folder" :size="22" /><b>{{ f }}</b><span class="chev" aria-hidden="true">›</span>
      </button>
      <p v-if="!loading && !folders.length && !trouble" class="none">No folders inside. You can use this one.</p>
    </div>
    <PTrouble v-if="trouble" :error="trouble" :where="where" />
  </div>
</template>

<style scoped>
.picker { display: flex; flex-direction: column; gap: 10px; }
.where { display: flex; align-items: center; gap: 10px; min-width: 0; }
.up { flex: none; height: 40px; padding: 0 12px; font: inherit; font-size: 15px; font-weight: 800; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 3px 3px 0 var(--edge); }
.up:disabled { opacity: 0.4; box-shadow: none; }
.path { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 15px; font-weight: 800; }
.list { overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.wait { display: block; margin: 16px 14px; }
.row { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 56px; padding: 8px 14px; font: inherit; text-align: left; color: var(--text); background: var(--panel); border: 0; border-top: 2px solid var(--edge); }
.row:first-of-type { border-top: 0; }
.row:active { background: var(--surface-hover); }
.row b { flex: 1; min-width: 0; font-size: 16px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.chev { flex: none; font-size: 22px; font-weight: 800; }
.none { margin: 0; padding: 14px; font-size: 15px; color: var(--text-quiet); }
</style>
