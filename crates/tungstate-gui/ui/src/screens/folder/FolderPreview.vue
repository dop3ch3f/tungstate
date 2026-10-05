<!-- What would happen, before anything does; then the two buttons.
     Safety properties 1 to 5 all live on this screen. -->
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { sentence } from "../../lib/format";
import { useFolders } from "../../state/useFolders";
import { useWatch } from "../../state/useWatch";
import { folders } from "../../engine/commands";
import { bytes, duration } from "../../lib/format";
import { wouldMove, outOf, moved, skipped, failed, putBack, files, dirsCounted } from "../../lib/counts";
import { ask } from "../../ui/useDialog";
import type { TreeEntry } from "../../engine/types";
import Button from "../../ui/Button.vue";
import ActionBar from "../../ui/ActionBar.vue";
import Tile from "../../ui/Tile.vue";
import BackLink from "../../ui/BackLink.vue";
import Notice from "../../ui/Notice.vue";
import Working from "../../ui/Working.vue";
import SortHead from "../../ui/SortHead.vue";
import TableTools from "../../ui/TableTools.vue";
import { useTable } from "../../lib/table";

const f = useFolders();
const w = useWatch();

type View = "trees" | "now" | "moves" | "alone" | "ruletext";
const view = ref<View>("trees");
const rules = ref<string | null>(null);

const p = computed(() => f.preview.value);

const top = (path: string) => (path.includes("/") ? path.slice(0, path.indexOf("/")) : "(top)");
const moveTable = useTable(computed(() => p.value?.moves ?? []), {
  columns: [
    { key: "from", value: (m) => m.from },
    { key: "to", value: (m) => m.to },
  ],
  text: (m) => `${m.from} ${m.to} ${m.why}`,
  facets: [
    { key: "into", label: "Into", of: (m) => top(m.to) },
  ],
});
const aloneTable = useTable(computed(() => p.value?.left_alone ?? []), {
  columns: [{ key: "path", value: (a) => a.path }],
  text: (a) => `${a.path} ${a.why}`,
  facets: [{ key: "why", label: "Reason", of: (a) => a.why }],
});
/** The folder's own name, not `p.folder`, which is the name the policy file
 *  declares. A folder called `messy-downloads` under the `downloads` layout
 *  was rendering as "downloads", on the page and in the question. */
const name = computed(() => f.current.value?.name ?? f.root.value?.split("/").pop() ?? "");
/** What the watcher has noticed arriving in this folder and not filed.
 *
 *  Only for folders that do not file things on their own: in enforce the
 *  watcher has already dealt with it, and saying "4 files arrived" about files
 *  that are no longer there would be a lie with a timestamp on it. */
const arrived = computed(() => {
  const here = name.value;
  return w.recent.value.find(
    (notice) => notice.kind === "waiting" && notice.folder === here,
  );
});

/** `PreviewView` has no `created`/`removed`, but both trees mark their
 *  directories, so the two numbers are a comparison rather than a missing
 *  field. Property 5 says they matter as much as files moved. */
const dirs = computed(() => {
  const view = p.value;
  if (!view) return { made: dirsCounted(0), emptied: dirsCounted(0) };
  const was = new Set(view.before.filter((e) => e.is_dir).map((e) => e.path));
  const now = new Set(view.after.filter((e) => e.is_dir).map((e) => e.path));
  let made = 0;
  let emptied = 0;
  for (const path of now) if (!was.has(path)) made += 1;
  for (const path of was) if (!now.has(path)) emptied += 1;
  return { made: dirsCounted(made), emptied: dirsCounted(emptied) };
});

/** Nothing would move: before and after are the same tree, and the move list
 *  is empty, so only what stays put and the rules are worth a tab. */
const still = computed(() => !!p.value && (p.value.tidy || !p.value.settles));
watch(still, (now) => {
  if (now && (view.value === "trees" || view.value === "moves")) view.value = "now";
  if (!now && view.value === "now") view.value = "trees";
}, { immediate: true });
/** Below this many rows, a filter is more to read than to use. */
const FEW = 12;
/** Just tidied, and it did something: the result leads, not the next plan. */
const justTidied = computed(() => !!f.tidied.value && !f.tidied.value.already_tidy);

watch(
  () => [view.value, f.root.value] as const,
  async ([v, root]) => {
    if (v !== "ruletext" || !root || rules.value !== null) return;
    rules.value = await folders.rulesText(root).catch((e) => String(e));
  },
);
watch(() => f.root.value, () => (rules.value = null));

/** Files at the top of the folder first, then each directory with what is in
 *  it. The engine sends plain path order, where `Images` sorts before
 *  `archive.zip`; drawn with directories as headings, that put top-level
 *  files visually inside `Images`. */
const topFirst = (entries: TreeEntry[]) =>
  [...entries].sort((a, b) => {
    const at = !a.is_dir && !a.path.includes("/");
    const bt = !b.is_dir && !b.path.includes("/");
    return at === bt ? (a.path < b.path ? -1 : a.path > b.path ? 1 : 0) : at ? -1 : 1;
  });
const before = computed(() => topFirst(p.value?.before ?? []));
const after = computed(() => topFirst(p.value?.after ?? []));
/** Whether a tree has files at its top as well as folders: then the top
 *  files get a heading of their own, the way each folder has one. */
const mixed = (entries: TreeEntry[]) =>
  entries.some((e) => e.is_dir) && entries.some((e) => !e.is_dir && !e.path.includes("/"));
const beforeMixed = computed(() => mixed(before.value));
const afterMixed = computed(() => mixed(after.value));

/** A path split for display: the directories, then the name. */
const cut = (path: string) => path.lastIndexOf("/") + 1;
const leaf = (path: string) => path.slice(cut(path));
/** A file under a directory heading needs only its name; the heading says
 *  where it is. A directory heading is shown whole. */
const label = (e: TreeEntry) => (e.is_dir ? e.path : leaf(e.path));
const nested = (e: TreeEntry) => !e.is_dir && e.path.includes("/");

/** Where a move is a copy and a delete, what that costs, in one sentence. */
const copyLine = computed(() => {
  const c = p.value?.copies;
  if (!c) return null;
  return c.on_server
    ? `This storage cannot rename, so each file is copied on the server and then the original deleted: ${bytes(c.bytes)} in all.`
    : `This storage cannot rename, so each file comes down to this computer, goes back up, and then the original is deleted: ${bytes(c.bytes)} each way.`;
});

async function confirmTidy() {
  const view = p.value;
  if (!view) return;
  const undo = "Everything it does can be put back from this screen.";
  const answer = await ask({
    title: `Tidy ${name.value}?`,
    why: [
      view.large ? `That is a large change: ${files(wouldMove(view))} of ${outOf(view)} move.` : "",
      copyLine.value ?? "",
      undo,
    ].filter(Boolean).join(" "),
    choices: [
      { id: "no", label: "Not now" },
      { id: "yes", label: "Tidy up", look: "primary" },
    ],
  });
  if (answer.id === "yes") await f.tidy();
}

async function confirmPutBack() {
  const plan = p.value?.undoable;
  if (plan == null) return;
  const answer = await ask({
    title: "Put it back?",
    why: "Every file this reorganisation moved goes back where it was. It refuses rather than overwriting anything you have changed since.",
    choices: [
      { id: "no", label: "Leave it" },
      { id: "yes", label: "Put it back", look: "primary" },
    ],
  });
  if (answer.id === "yes") await f.putBack(plan);
}
</script>

<template>
  <div class="prev" v-if="p">
    <div class="column">
      <BackLink @back="f.back()">All folders</BackLink>
      <div class="head"><Tile of="folder" :size="26" /><h1>{{ name }}</h1></div>
      <p class="locus">
        <span class="path">{{ f.root.value }}</span>
        <span class="ruleset" v-if="p.folder">filed by the {{ p.folder }} rules</span>
      </p>

      <Notice tone="plain" v-if="f.tidied.value?.already_tidy" class="did">
        Nothing to do: this folder already matches its rules.
      </Notice>
      <Notice tone="hold" v-if="arrived">
        {{ arrived.files }} file{{ arrived.files === 1 ? "" : "s" }} arrived while you
        were away. Tidy up files {{ arrived.files === 1 ? "it" : "them" }}.
      </Notice>

      <Notice tone="bad" v-if="f.problem.value">{{ sentence(f.problem.value) }}</Notice>
      <Notice tone="hold" v-if="copyLine && !justTidied">{{ copyLine }}</Notice>

      <!-- Property 1 and 5: what would move, and what would happen to the
           shape, before any button exists. -->
      <!-- What the tidy just did, worded from TidyDone and nothing else. -->
      <div class="sum hero result" v-if="justTidied">
        <p class="big num">{{ files(moved(f.tidied.value!)) }} moved</p>
        <p class="sub">
          <template v-if="f.tidied.value!.skipped">
            {{ files(skipped(f.tidied.value!)) }} changed while we looked, so
            {{ f.tidied.value!.skipped === 1 ? "it was" : "they were" }} skipped.
          </template>
          <template v-if="f.tidied.value!.failed">
            {{ files(failed(f.tidied.value!)) }} could not be moved.
          </template>
          Put it back undoes all of it.
        </p>
      </div>
      <div class="sum hero result" v-else-if="f.putBackCount.value !== null">
        <p class="big num">{{ f.putBackCount.value }} file{{ f.putBackCount.value === 1 ? "" : "s" }} put back</p>
        <p class="sub">The folder is as it was before that tidy. Tidy up would move {{ files(wouldMove(p)) }} again.</p>
      </div>
      <p class="sum" v-else-if="!p.settles">
        These rules never settle: they would keep moving the same files every
        run. Nothing can be tidied until the rules are fixed.
      </p>
      <p class="sum" v-else-if="p.tidy && p.waiting">
        Nothing to move yet. {{ p.waiting }} file{{ p.waiting === 1 ? " is" : "s are" }}
        still too recent to touch; the longest has {{ duration(p.longest_wait) }} to wait.
      </p>
      <p class="sum" v-else-if="p.tidy">
        Nothing to move. This folder already matches its rules.
      </p>
      <div class="sum hero" v-else>
        <p class="big num">{{ wouldMove(p) }} of {{ files(outOf(p)) }} would move</p>
        <p class="sub">
          {{ bytes(p.bytes) }} ·
          {{ dirs.made }} new folder{{ dirs.made === 1 ? "" : "s" }}<template v-if="dirs.emptied">,
          {{ dirs.emptied }} emptied</template>
        </p>
      </div>

      <nav class="views">
        <button v-if="!still" :class="{ von: view === 'trees' }" @click="view = 'trees'">Before and after</button>
        <button v-if="still" :class="{ von: view === 'now' }" @click="view = 'now'">The folder now</button>
        <button v-if="!still" :class="{ von: view === 'moves' }" @click="view = 'moves'">Every move</button>
        <button :class="{ von: view === 'alone' }" @click="view = 'alone'">
          Left alone <span class="howmany">{{ p.left_alone.length }}</span>
        </button>
        <button :class="{ von: view === 'ruletext' }" @click="view = 'ruletext'">Rules</button>
      </nav>

      <div class="view-body">
        <!-- Nothing would move, so before and after are one tree: show it once. -->
        <div class="trees one" v-if="view === 'now'">
          <div class="side">
            <ul>
              <li class="dir" v-if="afterMixed"><span class="entry">At the top</span></li>
              <li v-for="e in after" :key="'n' + e.path" :class="{ dir: e.is_dir, nested: nested(e) || (afterMixed && !e.is_dir) }">
                <span class="entry">{{ label(e) }}</span>
                <span class="weight" v-if="!e.is_dir">{{ bytes(e.size) }}</span>
              </li>
            </ul>
          </div>
        </div>
        <div class="trees" v-else-if="view === 'trees'">
          <div class="side">
            <h2>Now</h2>
            <ul>
              <li class="dir" v-if="beforeMixed"><span class="entry">At the top</span></li>
              <li v-for="e in before" :key="'b' + e.path" :class="{ shifts: e.moves, dir: e.is_dir, nested: nested(e) || (beforeMixed && !e.is_dir) }">
                <span class="entry">{{ label(e) }}</span>
                <span class="weight" v-if="!e.is_dir">{{ bytes(e.size) }}</span>
              </li>
            </ul>
          </div>
          <div class="side">
            <h2>After tidying</h2>
            <ul>
              <li class="dir" v-if="afterMixed"><span class="entry">At the top</span></li>
              <li v-for="e in after" :key="'a' + e.path" :class="{ shifts: e.moves, dir: e.is_dir, nested: nested(e) || (afterMixed && !e.is_dir) }">
                <span class="entry">{{ label(e) }}</span>
                <span class="weight" v-if="!e.is_dir">{{ bytes(e.size) }}</span>
              </li>
            </ul>
          </div>
        </div>

        <div v-else-if="view === 'moves'">
        <TableTools v-if="p.moves.length > FEW" :table="moveTable" placeholder="Filter the moves" />
        <div class="mhead" v-if="p.moves.length">
          <SortHead :table="moveTable" column="from">From</SortHead>
          <SortHead :table="moveTable" column="to">To</SortHead>
        </div>
        <p class="hint" v-if="p.moves.length && !moveTable.shown.value.length">Nothing here matches that.</p>
        <ul class="moves">
          <li v-for="m in moveTable.shown.value" :key="m.from">
            <span class="path">{{ m.from }}</span>
            <span class="becomes" aria-label="becomes">→</span>
            <span class="path to">{{ m.to }}</span>
          </li>
        </ul>
        </div>

        <!-- Property 4. The engine flattens eight reasons into one sentence
             before they cross the seam, so the window shows the sentence it
             wrote for each file rather than guessing at groups. Nothing is
             collapsed into a single word, which is what the property asks. -->
        <div class="alone" v-else-if="view === 'alone'">
          <TableTools v-if="p.left_alone.length > FEW" :table="aloneTable" placeholder="Filter what stays put" />
          <p class="hint" v-if="p.left_alone.length && !aloneTable.shown.value.length">Nothing here matches that.</p>
          <ul>
            <li v-for="a in aloneTable.shown.value" :key="a.path">
              <span class="path">{{ a.path }}</span>
              <span class="reason">{{ a.why }}</span>
            </li>
          </ul>
          <p class="hint" v-if="!p.left_alone.length">Every file here is claimed by a rule.</p>
        </div>

        <pre class="ruletext mono" v-else>{{ rules ?? "Reading the rules…" }}</pre>
      </div>

      <!-- Property 1: this button exists only here, beside the thing it
           would do. Property 2: the way back sits next to it, and stays
           visible when there is nothing to undo. The filled button is
           whichever one there is something to do with: Tidy up while files
           would move, Put it back after. -->
      <ActionBar class="foot">
        <template #say>
          <Working v-if="f.busy.value" :what="f.busy.value" note="you can leave this running" />
          <span v-else-if="p.undoable !== null && !justTidied">The last tidy of this folder can be undone.</span>
        </template>
        <template v-if="!f.busy.value">
          <!-- Undo is never the main button: a reflex click must not undo. -->
          <Button
            :disabled="p.undoable === null"
            :title="p.undoable === null ? 'Nothing here has been tidied yet' : undefined"
            @click="confirmPutBack()"
          >Put it back</Button>
          <Button
            v-if="!still"
            look="primary"
            :disabled="!!f.problem.value"
            @click="confirmTidy()"
          >Tidy up</Button>
        </template>
      </ActionBar>
    </div>
  </div>
</template>

<style scoped>
.head { display: flex; align-items: center; gap: var(--s3); }
.prev { position: absolute; inset: 0; overflow: hidden; }
.column {
  height: 100%;
  padding: var(--win-pad);
  padding-bottom: 0;
  display: flex;
  flex-direction: column;
}
h1 { font-size: var(--display); font-weight: 700; margin: 0; letter-spacing: -0.01em; }
.locus { display: flex; gap: var(--s3); align-items: baseline; margin: var(--s1) 0 0; }
.ruleset { font-size: var(--fine); color: var(--text-faint); flex: none; }
.locus .path { color: var(--text-faint); }
.did { margin-top: var(--s3); }

.sum { font-size: var(--body); line-height: 1.55; margin: var(--s4) 0 0; color: var(--text-quiet); }
.hero { display: flex; align-items: baseline; gap: var(--s1) var(--s3); flex-wrap: wrap; }
.big { font-size: var(--hero); font-weight: 700; letter-spacing: -0.02em; line-height: 1.1; color: var(--text); margin: 0; }
.sub { margin: 0; max-width: 64ch; }
/* A result's caption is a sentence, so it goes under its headline. */
.result .sub { flex-basis: 100%; }

.views { display: flex; gap: var(--s1); margin: var(--s4) calc(var(--s2) * -1) 0; }
.views button {
  font: inherit;
  font-size: var(--small);
  background: none;
  border: none;
  color: var(--text-faint);
  padding: var(--s1) var(--s2);
  border-radius: var(--radius);
  cursor: pointer;
}
.views button:hover { color: var(--text-quiet); }
.views .von { color: var(--text); background: var(--surface-raised); }
.views { padding: 3px; background: var(--rail); border-radius: var(--radius); align-self: flex-start; margin-left: 0; }
.views button { border-radius: var(--radius); padding: var(--s1) var(--s3); }

/* Retro: a tab is a block with an outline, and the one you are in is white and bold, like the section in the rail. */
/* Retro: one joined strip in an outline, no shadow, so a tab never reads
   as a button; the one you are in is filled and bold, like the rail. */
:global([data-theme="retro"] .views) {
  background: var(--surface-raised);
  padding: 0;
  gap: 0;
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius);
  overflow: hidden;
}
:global([data-theme="retro"] .views button) {
  border: none;
  border-right: var(--bw) solid var(--edge);
  border-radius: 0;
  background: none;
  color: var(--text);
  padding: 5px var(--s3);
}
:global([data-theme="retro"] .views button:last-child) { border-right: none; }
:global([data-theme="retro"] .views .von) {
  background: var(--chosen);
  color: var(--chosen-ink);
  font-weight: 700;
}
.howmany { font-variant-numeric: tabular-nums; opacity: 0.7; margin-left: 2px; }

/* The list scrolls on its own, so it stops above the action bar instead of
   running underneath it. */
.view-body {
  margin-top: var(--s3);
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-bottom: var(--s5);
}

.trees { display: grid; grid-template-columns: 1fr 1fr; gap: var(--s5); }
.trees.one { grid-template-columns: minmax(0, 36rem); }
.side h2 { font-size: var(--fine); font-weight: 600; color: var(--text-faint); margin: 0 0 var(--s2); }
.side ul, .moves, .alone ul { list-style: none; margin: 0; padding: 0; }
.side li {
  display: flex;
  justify-content: space-between;
  gap: var(--s3);
  font-size: var(--fine);
  padding: 2px 0;
  color: var(--text-quiet);
}
/* The files that would change are the bright ones, each with an accent mark,
   so the eye can follow them across. Everything else steps back. */
.side li { color: var(--text-faint); padding-left: 12px; position: relative; }
.side li.shifts { color: var(--text); }
.side li.shifts::before {
  content: "";
  position: absolute;
  left: 0;
  top: 50%;
  width: 5px;
  height: 5px;
  margin-top: -2px;
  border-radius: 50%;
  background: var(--accent);
}
.side li.dir { padding-left: 0; margin-top: var(--s2); color: var(--text-quiet); }
.side li.dir .entry { font-size: var(--small); font-weight: 600; }
.entry { font-size: var(--small); min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.side li.nested { padding-left: 24px; }
.side li.nested.shifts::before { left: 12px; }
.weight { font-variant-numeric: tabular-nums; flex: none; color: var(--text-faint); }

.moves li, .alone li {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0 var(--s2);
  padding: var(--s1) 0;
  font-size: var(--fine);
  color: var(--text-quiet);
  border-bottom: 1px solid var(--rule);
}
.becomes { font-size: var(--small); color: var(--text-faint); }
.mhead { display: flex; gap: var(--s5); font-size: var(--fine); color: var(--text-faint); padding: 4px 0; border-bottom: 1px solid var(--rule); }
:global([data-theme="retro"] .mhead) { background: var(--surface-raised); color: var(--text); font-weight: 700; font-family: var(--font-mono); padding: 5px var(--s2); }
.to { color: var(--text); }
.alone .reason { color: var(--text-faint); margin-left: auto; }
.hint { font-size: var(--small); color: var(--text-quiet); margin: 0 0 var(--s2); }
.ruletext {
  font-size: var(--fine);
  line-height: 1.6;
  margin: 0;
  white-space: pre-wrap;
  color: var(--text-quiet);
}

.foot { margin-top: auto; flex: none; }
</style>
