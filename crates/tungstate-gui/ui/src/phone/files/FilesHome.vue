<!-- Files home: the phone's storage first, then its files by kind, your
     connections, and where you were lately. Every tile opens a file window.
     Matches docs/mobile/approved-files/home.png. Until All files access is
     on, it asks for that instead: without it there is nothing to show. -->
<script setup lang="ts">
import { computed, onMounted } from "vue";
import { bytes } from "../../lib/format";
import { useConnections } from "../../state/useConnections";
import { usePhoneNav } from "../nav";
import { CATEGORIES } from "../lib/look";
import { connectionOf, nameOf, parentOf, topOf } from "../lib/paths";
import { useStorage } from "../state/useStorage";
import { useWindows } from "../state/useWindows";
import PButton from "../ui/PButton.vue";
import PTile from "../ui/PTile.vue";
import ClipBar from "./ClipBar.vue";

const store = useStorage();
const conns = useConnections();
const w = useWindows();
const nav = usePhoneNav();
onMounted(() => {
  void store.load();
  void conns.load();
});

const reachable = computed(() => conns.all.value.filter((c) => c.scheme !== "fs"));
const used = (v: { total: number; free: number }) => (v.total ? Math.round(((v.total - v.free) / v.total) * 100) : 0);
/** A recent folder's name: a volume's top is called by the volume's name. */
function titleOf(path: string): string {
  const top = topOf(path, store.tops.value);
  return top && top.path.replace(/\/+$/, "") === path.replace(/\/+$/, "") ? top.label : nameOf(path) || path;
}

/** "Internal › DCIM": where a recent folder sits, under its top's name. */
function within(path: string): string {
  if (titleOf(path) !== nameOf(path)) return "";
  const up = parentOf(path);
  if (!up) return "";
  const top = topOf(up, store.tops.value);
  if (connectionOf(up) !== null) return up.replace(/:$/, "").replace(":", " › ").replaceAll("/", " › ");
  const rest = top ? up.slice(top.path.length).split("/").filter(Boolean) : [nameOf(up)];
  return [top?.label ?? "", ...rest].filter(Boolean).join(" › ");
}
</script>

<template>
  <section class="home">
    <div class="head">
      <h1>Files</h1>
    </div>
    <div class="body">
      <p v-if="store.problem.value" class="line bad">{{ store.problem.value }}</p>

      <div v-if="store.access.value && store.access.value !== 'full'" class="card">
        <PTile art="folder" tint="var(--tint-home)" :size="48" />
        <b>Let Tungstate use your files</b>
        <p>
          To browse this phone, and copy and move files to your connections. Android calls this All files access;
          Tungstate never reads a file you have not opened or chosen.
        </p>
        <PButton tint="var(--tint-drain)" @press="store.ask()">Allow access</PButton>
      </div>

      <div v-if="store.volumes.value.length" class="list">
        <button v-for="v in store.volumes.value" :key="v.path" class="row" @click="w.go({ kind: 'folder', path: v.path })">
          <PTile :art="v.removable ? 'sd' : 'phone'" :tint="v.removable ? '#a5a79d' : 'var(--tint-home)'" :size="40" />
          <span class="text">
            <b>{{ v.label }}</b>
            <span v-if="v.total" class="meter" aria-hidden="true"><i :style="{ width: `${used(v)}%` }"></i></span>
            <span class="quiet">{{ v.total ? `${bytes(v.free)} free of ${bytes(v.total)}` : "Size not known" }}</span>
          </span>
          <span class="chev" aria-hidden="true">›</span>
        </button>
      </div>

      <template v-if="store.access.value === 'full'">
        <h2>On this phone</h2>
        <div class="cats">
          <button v-for="c in CATEGORIES" :key="c.id" class="cat" @click="w.go({ kind: 'category', category: c.id })">
            <PTile :art="c.art" :tint="c.tint" :size="40" />
            <b>{{ c.label }}</b>
            <span class="quiet">{{ store.categories.value ? bytes(store.categories.value[c.id].bytes) : "…" }}</span>
          </button>
        </div>
      </template>

      <h2>Connections</h2>
      <div class="conns">
        <button v-for="c in reachable" :key="c.name" class="conn" @click="w.go({ kind: 'folder', path: `${c.name}:` })">
          <PTile art="nas" tint="var(--tint-connections)" light :size="32" />{{ c.name }}
        </button>
        <button class="conn add" @click="nav.go('connections')">+ Add</button>
      </div>

      <template v-if="store.recent.value.length">
        <h2>Recent</h2>
        <div class="list">
          <button v-for="p in store.recent.value" :key="p" class="row tight" @click="w.go({ kind: 'folder', path: p })">
            <PTile art="folder" tint="var(--raw-kind-folder)" />
            <span class="text"><b>{{ titleOf(p) }}</b><span class="quiet">{{ within(p) }}</span></span>
            <span class="chev" aria-hidden="true">›</span>
          </button>
        </div>
      </template>
    </div>
    <ClipBar :folder="null" />
  </section>
</template>

<style scoped>
.home { flex: 1; min-height: 0; display: flex; flex-direction: column; }
.head { flex: none; display: flex; align-items: center; justify-content: space-between; padding: 10px 20px 12px; }
h1 { margin: 0; font-size: 32px; line-height: 1.08; font-weight: 800; letter-spacing: -0.6px; }
h2 { margin: 16px 0 8px; font-size: 15px; font-weight: 800; }
.body { flex: 1; min-height: 0; overflow-y: auto; padding: 0 20px 20px; }
.line { margin: 4px 2px 12px; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.bad { color: var(--bad); font-weight: 700; }
.quiet { font-size: 14px; color: var(--text-quiet); }
.card { display: flex; flex-direction: column; gap: 12px; margin-bottom: 14px; padding: 18px; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.card b { font-size: 19px; }
.card p { margin: 0; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.list { overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.row { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 70px; padding: 9px 14px; font: inherit; text-align: left; color: var(--text); background: var(--panel); border: 0; border-top: 2px solid var(--edge); }
.row:first-child { border-top: 0; }
.row:active, .cat:active { background: var(--surface-hover); }
.row.tight { min-height: 64px; }
.text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
.text b { font-size: 16px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.chev { flex: none; font-size: 22px; font-weight: 800; }
.meter { display: block; height: 12px; overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 4px; }
.meter i { display: block; height: 100%; background: var(--tint-drain); border-right: 2px solid var(--edge); }
.cats { display: grid; grid-template-columns: repeat(3, 1fr); overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.cat { display: flex; flex-direction: column; align-items: center; gap: 5px; padding: 9px 4px 8px; font: inherit; color: var(--text); background: var(--panel); border: 0; border-right: 2px solid var(--edge); border-bottom: 2px solid var(--edge); }
.cat:nth-child(3n) { border-right: 0; }
.cat:nth-child(n + 4) { border-bottom: 0; }
.cat b { font-size: 14px; }
.cat .quiet { margin-top: -3px; font-size: 13px; }
.conns { display: flex; gap: 8px; }
.conn { flex: 1; min-width: 0; display: flex; align-items: center; gap: 8px; height: 52px; padding: 0 12px 0 8px; overflow: hidden; font: inherit; font-size: 15px; font-weight: 800; white-space: nowrap; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 3px 3px 0 var(--edge); }
.conn.add { flex: none; justify-content: center; padding: 0 14px; }
.conn:active { transform: translate(3px, 3px); box-shadow: none; }
</style>
