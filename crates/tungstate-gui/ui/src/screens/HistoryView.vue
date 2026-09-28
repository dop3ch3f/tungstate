<!-- Everything that has happened, and where one file went.
     Activity and Find were two screens returning the same `Op[]`. The only
     difference was that Find also accepts a BLAKE3 digest, which is one line,
     so they are one screen. -->
<script setup lang="ts">
import { computed, onMounted, ref, shallowRef, watch } from "vue";
import { history } from "../engine/commands";
import { bytes, shortPath, when } from "../lib/format";
import { toneOfStatus } from "../lib/tone";
import { latest } from "../lib/latest";
import type { Op } from "../engine/types";
import Button from "../ui/Button.vue";
import Tile from "../ui/Tile.vue";
import Notice from "../ui/Notice.vue";
import Empty from "../ui/Empty.vue";
import SortHead from "../ui/SortHead.vue";
import TableTools from "../ui/TableTools.vue";
import { useTable } from "../lib/table";

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

// A second Look before the first answers wins, whichever returns first.
const asks = latest();
const looking = ref(false);

async function look() {
  const target = asked.value.trim();
  if (!target) return recent();
  searched.value = true;
  looking.value = true;
  const ticket = asks.take();
  try {
    const found = isDigest(target) ? await history.whereis(target) : await history.ofPath(target);
    if (asks.stale(ticket)) return;
    ops.value = found;
    problem.value = null;
  } catch (e) {
    if (!asks.stale(ticket)) problem.value = String(e);
  } finally {
    if (!asks.stale(ticket)) looking.value = false;
  }
}

const DOT = { plain: "h-plain", live: "h-live", ok: "h-ok", hold: "h-hold", bad: "h-bad" } as const;
const dot = (status: string) => DOT[toneOfStatus(status)];

/** `interrupted` means the intent was written and the outcome never was, which
 *  is a different thing from a failure and is said differently. */
// Only what went wrong is said in words; a done operation's dot says done.
const VERDICT: Record<string, string> = {
  ok: "",
  interrupted: "interrupted",
  failed: "failed",
  skipped: "skipped",
};

const leaf = (path: string | null) => (path ?? "").split("/").pop() ?? "";
const verdict = (op: Op) => VERDICT[op.status] ?? op.status;

// Narrows the rows already here. Finding a file's whole history is the form
// above, which asks the journal; this never does.
const table = useTable(ops, {
  columns: [
    { key: "kind", value: (op) => op.kind },
    { key: "file", value: (op) => leaf(op.destination ?? op.source) },
    { key: "outcome", value: (op) => (op.status === "ok" ? "done" : verdict(op)) },
    { key: "size", value: (op) => op.size },
    { key: "when", value: (op) => op.started_at },
  ],
  text: (op) => `${op.source ?? ""} ${op.destination ?? ""} ${op.link ?? ""}`,
  facets: [
    { key: "kind", label: "Kind", of: (op) => op.kind },
    // The filter names every outcome, done included, though the column says
    // only what went wrong.
    { key: "outcome", label: "Outcome", of: (op) => (op.status === "ok" ? "done" : verdict(op)) },
  ],
  sort: { key: "when", dir: -1 },
});
const dir = (path: string | null) => (path ?? "").slice(0, (path ?? "").lastIndexOf("/"));

/** What an operation changed: the name, if it stayed in its folder, or the
 *  folder it moved between. */
function change(op: Op): string {
  const from = op.source;
  const to = op.destination ?? op.source;
  if (!from || !op.destination) return tail(dir(to));
  if (dir(from) === dir(op.destination)) return `${leaf(from)} → ${leaf(op.destination)}`;
  // Only the folders after where the two paths part: the shared front says
  // nothing, and it is what used to push the answer off the end.
  const a = dir(from).split("/");
  const b = dir(op.destination).split("/");
  let same = 0;
  while (same < a.length && same < b.length && a[same] === b[same]) same += 1;
  // Two different places share nothing; their last folders tell them apart.
  if (same <= 1) return `${tail(dir(from))} → ${tail(dir(op.destination))}`;
  const left = a.slice(same).join("/");
  const right = b.slice(same).join("/");
  return left ? `${left} → ${right || "(up a level)"}` : `into ${right}`;
}
/** The last two folders of a path, which is what tells places apart. */
function tail(path: string): string {
  const parts = path.split("/").filter(Boolean);
  const kept = parts.slice(-2).join("/");
  return parts.length > 2 ? `…/${kept}` : path;
}

/** Where a searched-for file is now: the last place anything put it. */
const now = computed(() => {
  const placed = ops.value.filter((op) => op.destination).sort((a, b) => b.started_at - a.started_at);
  return placed[0]?.destination ?? null;
});

// Rows fall under their day while the list runs by time, so each row says
// only the time and "28 Sept" is not repeated down the page.
const byDay = computed(() => table.sort.value.key === "when");
const day = (ms: number) => new Date(ms).toDateString();
function dayLabel(ms: number): string {
  const today = new Date();
  const yesterday = new Date(today.getTime() - 86_400_000);
  if (day(ms) === today.toDateString()) return "Today";
  if (day(ms) === yesterday.toDateString()) return "Yesterday";
  return new Date(ms).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "short" });
}
const clock = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });

// A found file's story reads oldest first; the recent list, newest first.
watch(searched, (on) => {
  table.sort.value = { key: "when", dir: on ? 1 : -1 };
});
</script>

<template>
  <div class="h-wrap">
    <div class="h-column">
      <div class="head"><Tile of="history" :size="26" /><h1>History</h1></div>
      <p class="h-lede">Everything Tungstate has done to your files. Search to follow one.</p>
      <form class="h-find" @submit.prevent="look()">
        <input
          id="h-in"
          v-model="asked"
          class="h-in"
          aria-label="A file's path or fingerprint"
          placeholder="A file's path or fingerprint"
        />
        <Button :busy="looking">Find</Button>
        <Button look="link" v-if="searched" @click="asked = ''; recent()">Show everything recent</Button>
      </form>
      <p class="h-mode" v-if="asked.trim()">
        {{ isDigest(asked.trim()) ? "Searching by fingerprint: every copy of this content, wherever it went." : "Searching by path." }}
      </p>

      <Notice tone="bad" v-if="problem">{{ problem }}</Notice>

      <Empty v-if="!ops.length && searched" art="nothing-found" line="Tungstate has not touched a file at that path.">
        <Button look="link" @click="asked = ''; recent()">Show everything recent</Button>
      </Empty>
      <Empty v-else-if="!ops.length" art="no-history" line="Nothing has happened yet." />
      <div class="h-answer" v-if="searched && now">
        <p class="h-now"><b>{{ leaf(now) }}</b> is now in</p>
        <p class="path h-where" :title="now"><bdi>{{ shortPath(dir(now)) }}</bdi></p>
      </div>

      <!-- A found file's story is short; filters earn their room on the long list. -->
      <TableTools v-if="ops.length > 6 && !searched" :table="table" placeholder="Filter these rows" />
      <div class="h-head" v-if="ops.length">
        <span></span>
        <SortHead :table="table" column="kind">Kind</SortHead>
        <SortHead :table="table" column="file">File</SortHead>
        <SortHead :table="table" column="size" numeric>Size</SortHead>
        <SortHead :table="table" column="when" numeric>When</SortHead>
      </div>
      <p class="h-nomatch" v-if="ops.length && !table.shown.value.length">Nothing here matches that.</p>
      <template v-for="(op, i) in table.shown.value" :key="op.id">
        <p class="h-day" v-if="byDay && (i === 0 || day(table.shown.value[i - 1]!.started_at) !== day(op.started_at))">
          {{ dayLabel(op.started_at) }}
        </p>
        <div class="h-row">
          <span class="h-dot" :class="dot(op.status)"></span>
          <span class="h-kind">{{ op.kind }}</span>
          <span class="h-what">
            <span class="h-leaf">{{ leaf(op.destination ?? op.source) }}</span>
            <span class="path h-change" :title="`${op.source ?? ''} → ${op.destination ?? ''}`">{{ change(op) }}</span>
            <span class="h-why" v-if="op.status !== 'ok'">{{ verdict(op) }}<template v-if="op.note">: {{ op.note }}</template></span>
          </span>
          <span class="num h-size">{{ op.size === null ? "" : bytes(op.size) }}</span>
          <span class="h-when">{{ byDay ? clock(op.started_at) : when(op.started_at) }}</span>
        </div>
      </template>
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
.h-lede { font-size: var(--body); color: var(--text-quiet); margin: 0 0 var(--s4); max-width: 70ch; line-height: 1.55; }
.h-nomatch { font-size: var(--small); color: var(--text-quiet); }
.h-mode { font-size: var(--fine); color: var(--text-faint); margin: calc(var(--s4) * -1) 0 var(--s4); }
/* A search's answer first: where the file is now. */
.h-answer { margin: 0 0 var(--s4); }
.h-now { font-size: var(--body); color: var(--text-quiet); margin: 0; }
.h-now b { color: var(--text); }
.h-where { font-size: var(--title); color: var(--text); margin: var(--s1) 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; }
.h-day { font-size: var(--small); font-weight: 700; margin: var(--s4) 0 0; padding: 0 var(--s2) var(--s1); border-bottom: 1px solid var(--rule); }

.h-head, .h-row { display: grid; }
.h-head {
  grid-template-columns: 9px 72px minmax(0, 1fr) 78px 118px;
  gap: var(--s2);
  padding: 4px var(--s2);
  font-size: var(--fine);
  color: var(--text-faint);
  border-bottom: 1px solid var(--rule);
}
:global([data-theme="retro"] .h-head) {
  background: var(--surface-raised);
  color: var(--text);
  font-weight: 700;
  font-family: var(--font-mono);
  padding: 5px var(--s2);
}
.h-row {
  display: grid;
  grid-template-columns: 9px 72px minmax(0, 1fr) 78px 118px;
  gap: var(--s2);
  align-items: center;
  /* Inset as the retro header band is, so rows and band share one edge. */
  padding: 7px var(--s2);
  font-size: var(--fine);
  border-bottom: 1px solid var(--rule);
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
.h-change { color: var(--text-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; word-break: normal; }
.h-why { color: var(--bad); font-size: var(--fine); font-weight: 600; }
.h-size, .h-when { color: var(--text-faint); text-align: right; }
</style>
