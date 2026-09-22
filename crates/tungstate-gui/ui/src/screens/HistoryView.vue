<!-- Everything that has happened, and where one file went.
     Activity and Find were two screens returning the same `Op[]`. The only
     difference was that Find also accepts a BLAKE3 digest, which is one line,
     so they are one screen. -->
<script setup lang="ts">
import { onMounted, ref, shallowRef } from "vue";
import { history } from "../engine/commands";
import { bytes, shortPath, when } from "../lib/format";
import { toneOfStatus } from "../lib/tone";
import type { Op } from "../engine/types";
import Button from "../ui/Button.vue";
import Tile from "../ui/Tile.vue";
import Notice from "../ui/Notice.vue";
import Empty from "../ui/Empty.vue";

const ops = shallowRef<Op[]>([]);
const asked = ref("");
const searched = ref(false);
const problem = ref<string | null>(null);

// A BLAKE3 digest is 64 hex characters and no real path looks like one. The
// same rule the engine uses, so the two surfaces cannot disagree.
const isDigest = (s: string) => s.length === 64 && /^[0-9a-f]+$/i.test(s);

async function recent() {
  searched.value = false;
  try {
    ops.value = await history.recent();
    problem.value = null;
  } catch (e) {
    problem.value = String(e);
  }
}
onMounted(recent);

async function look() {
  const target = asked.value.trim();
  if (!target) return recent();
  searched.value = true;
  try {
    ops.value = isDigest(target) ? await history.whereis(target) : await history.ofPath(target);
    problem.value = null;
  } catch (e) {
    problem.value = String(e);
  }
}

const DOT = { plain: "h-plain", live: "h-live", ok: "h-ok", hold: "h-hold", bad: "h-bad" } as const;
const dot = (status: string) => DOT[toneOfStatus(status)];

/** `interrupted` means the intent was written and the outcome never was, which
 *  is a different thing from a failure and is said differently. */
const VERDICT: Record<string, string> = {
  ok: "done",
  interrupted: "interrupted",
  failed: "failed",
  skipped: "skipped",
};

const leaf = (path: string | null) => (path ?? "").split("/").pop() ?? "";
const dir = (path: string | null) => (path ?? "").slice(0, (path ?? "").lastIndexOf("/"));
</script>

<template>
  <div class="h-wrap">
    <div class="h-column">
      <div class="head"><Tile of="history" :size="26" /><h1>What has happened</h1></div>
      <form class="h-find" @submit.prevent="look()">
        <input
          v-model="asked"
          class="h-in"
          placeholder="a path, or a file's fingerprint"
        />
        <Button>Look</Button>
        <Button look="link" v-if="searched" @click="asked = ''; recent()">Show everything recent</Button>
      </form>
      <p class="h-mode" v-if="asked.trim()">
        {{ isDigest(asked.trim()) ? "That is a fingerprint, so this searches by content." : "That is a path, so this searches by name." }}
      </p>

      <Notice tone="bad" v-if="problem">{{ problem }}</Notice>

      <Empty v-if="!ops.length && searched" line="Nothing here matches that." />
      <Empty v-else-if="!ops.length" line="Nothing has happened yet." />

      <div class="h-row" v-for="op in ops" :key="op.id" :class="{ 'h-row-bad': dot(op.status) === 'h-bad' }">
        <span class="h-dot" :class="dot(op.status)"></span>
        <span class="h-kind">{{ op.kind }}</span>
        <span class="h-what">
          <span class="h-leaf">{{ leaf(op.destination ?? op.source) }}</span>
          <span class="path h-dir"><bdi>{{ shortPath(dir(op.destination ?? op.source)) }}</bdi></span>
        </span>
        <span class="h-said" :class="{ 'h-said-bad': dot(op.status) === 'h-bad' }">{{ VERDICT[op.status] ?? op.status }}</span>
        <span class="num h-size">{{ op.size === null ? "" : bytes(op.size) }}</span>
        <span class="h-when">{{ when(op.started_at) }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.head { display: flex; align-items: center; gap: var(--s3); }
.h-wrap { position: absolute; inset: 0; overflow-y: auto; scrollbar-gutter: stable; }
.h-column { padding: var(--win-pad); }
h1 { font-size: var(--display); font-weight: 700; margin: 0; letter-spacing: -0.01em; }
.head { margin-bottom: var(--s4); }

.h-find { display: flex; gap: var(--s2); align-items: center; }
.h-in { min-width: 320px; }
.h-find { margin-bottom: var(--s5); }
.h-mode { font-size: var(--fine); color: var(--text-faint); margin: var(--s2) 0 0; }

.h-row {
  display: grid;
  grid-template-columns: 9px 72px minmax(0, 1fr) 90px 78px 118px;
  gap: var(--s2);
  align-items: center;
  padding: 7px 0;
  font-size: var(--fine);
  border-bottom: 1px solid var(--edge);
}
.h-dot { width: 7px; height: 7px; border-radius: 50%; }
.h-plain { background: var(--text-faint); }
.h-live { background: var(--accent); }
.h-ok { background: var(--ok); }
.h-hold { background: var(--hold); }
.h-bad { background: var(--bad); }
.h-kind { color: var(--text-faint); }
.h-what { display: flex; flex-direction: column; min-width: 0; }
.h-leaf { font-size: var(--small); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
/* Right-to-left so the ellipsis eats the front of the folder path and its
   nearest directories survive; `bdi` keeps the path itself reading left to right. */
.h-dir { color: var(--text-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; word-break: normal; direction: rtl; text-align: left; }
.h-said-bad { color: var(--bad); font-weight: 600; }
.h-row-bad { background: var(--panel); border-radius: var(--radius); }
.h-said, .h-size, .h-when { color: var(--text-faint); text-align: right; }
</style>
