<!-- Everything that has happened, and where one file went.
     Activity and Find were two screens returning the same `Op[]`. The only
     difference was that Find also accepts a BLAKE3 digest, which is one line,
     so they are one screen. -->
<script setup lang="ts">
import { onMounted, ref, shallowRef } from "vue";
import { history } from "../engine/commands";
import { bytes, when } from "../lib/format";
import { toneOfStatus } from "../lib/tone";
import type { Op } from "../engine/types";
import Button from "../ui/Button.vue";
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
</script>

<template>
  <div class="h-wrap">
    <div class="h-column">
      <h1>What has happened</h1>
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

      <div class="h-row" v-for="op in ops" :key="op.id">
        <span class="h-dot" :class="dot(op.status)"></span>
        <span class="h-kind">{{ op.kind }}</span>
        <span class="path h-what">{{ op.destination ?? op.source }}</span>
        <span class="h-said">{{ VERDICT[op.status] ?? op.status }}</span>
        <span class="num h-size">{{ op.size === null ? "" : bytes(op.size) }}</span>
        <span class="h-when">{{ when(op.started_at) }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.h-wrap { position: absolute; inset: 0; overflow-y: auto; scrollbar-gutter: stable; }
.h-column { max-width: 960px; margin: 0 auto; padding: var(--s5) var(--s6); }
h1 { font-size: var(--display); font-weight: 600; margin: 0 0 var(--s4); letter-spacing: -0.01em; }

.h-find { display: flex; gap: var(--s2); align-items: center; }
.h-in {
  font: inherit;
  font-size: var(--small);
  background: none;
  color: var(--text);
  border: 1px solid var(--edge);
  border-radius: var(--radius);
  padding: 6px 9px;
  min-width: 280px;
}
.h-mode { font-size: var(--fine); color: var(--text-faint); margin: var(--s2) 0 0; }

.h-row {
  display: grid;
  grid-template-columns: 9px 72px minmax(0, 1fr) 90px 78px 118px;
  gap: var(--s2);
  align-items: center;
  padding: 5px 0;
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
.h-what { color: var(--text-quiet); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; word-break: normal; }
.h-said, .h-size, .h-when { color: var(--text-faint); text-align: right; }
</style>
