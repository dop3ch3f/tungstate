<!-- What would happen, before anything does; then the two buttons.
     Safety properties 1 to 5 all live on this screen. -->
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useFolders } from "../../state/useFolders";
import { useWatch } from "../../state/useWatch";
import { folders } from "../../engine/commands";
import { bytes, duration } from "../../lib/format";
import { wouldMove, outOf, moved, skipped, failed, putBack, files, dirsCounted } from "../../lib/counts";
import { ask } from "../../ui/useDialog";
import type { TreeEntry } from "../../engine/types";
import Button from "../../ui/Button.vue";
import Tile from "../../ui/Tile.vue";
import Notice from "../../ui/Notice.vue";
import Working from "../../ui/Working.vue";

const f = useFolders();
const w = useWatch();

type View = "trees" | "moves" | "alone" | "ruletext";
const view = ref<View>("trees");
const rules = ref<string | null>(null);

const p = computed(() => f.preview.value);
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

/** A path split for display: the directories, then the name. */
const cut = (path: string) => path.lastIndexOf("/") + 1;
const lead = (path: string) => path.slice(0, cut(path));
const leaf = (path: string) => path.slice(cut(path));

async function confirmTidy() {
  const view = p.value;
  if (!view) return;
  const detail = [
    `${files(wouldMove(view))} would move, of ${outOf(view)}.`,
    `${bytes(view.bytes)} in all.`,
    `${dirs.value.made} directories made, ${dirs.value.emptied} emptied.`,
  ];
  const answer = await ask({
    title: `Tidy ${name.value}?`,
    why: view.large
      ? "This is a large change. Everything it does can be put back, and the button to do that stays on this screen."
      : "Everything this does can be put back, and the button to do that stays on this screen.",
    detail,
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
      <div class="head"><Tile of="folder" :size="26" /><h1>{{ name }}</h1></div>
      <p class="locus">
        <span class="path">{{ f.root.value }}</span>
        <span class="ruleset" v-if="p.folder">filed by the {{ p.folder }} rules</span>
      </p>

      <!-- What the tidy just did, worded from TidyDone and nothing else. -->
      <Notice tone="plain" v-if="f.tidied.value" class="did">
        <template v-if="f.tidied.value.already_tidy">
          Nothing to do: this folder already matches its rules.
        </template>
        <template v-else>
          Moved {{ files(moved(f.tidied.value)) }}.
          <template v-if="f.tidied.value.skipped">
            {{ files(skipped(f.tidied.value)) }} left alone:
            {{ f.tidied.value.skipped === 1 ? "it changed" : "they changed" }} while we looked.
          </template>
          <template v-if="f.tidied.value.failed">
            {{ files(failed(f.tidied.value)) }} could not be moved.
          </template>
        </template>
      </Notice>
      <Notice tone="hold" v-if="arrived">
        {{ arrived.files }} file{{ arrived.files === 1 ? "" : "s" }} arrived while you
        were away. Tidy up files {{ arrived.files === 1 ? "it" : "them" }}.
      </Notice>
      <Notice tone="plain" v-if="f.putBackCount.value !== null">
        Put {{ f.putBackCount.value }} file{{ f.putBackCount.value === 1 ? "" : "s" }} back.
      </Notice>
      <Notice tone="bad" v-if="f.problem.value">{{ f.problem.value }}</Notice>

      <!-- Property 1 and 5: what would move, and what would happen to the
           shape, before any button exists. -->
      <p class="sum" v-if="!p.settles">
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
        <p class="big num">{{ files(wouldMove(p)) }}</p>
        <p class="sub">
          of {{ outOf(p) }} would move, {{ bytes(p.bytes) }}.
          It would make {{ dirs.made }} director{{ dirs.made === 1 ? "y" : "ies" }}
          and empty {{ dirs.emptied }}.
        </p>
      </div>

      <Notice tone="hold" v-if="p.large && p.settles && !p.tidy">
        That is a large change for one folder. Read it before you run it.
      </Notice>

      <nav class="views">
        <button :class="{ von: view === 'trees' }" @click="view = 'trees'">Before and after</button>
        <button :class="{ von: view === 'moves' }" @click="view = 'moves'">
          Every move <span class="howmany">{{ p.moves.length }}</span>
        </button>
        <button :class="{ von: view === 'alone' }" @click="view = 'alone'">
          Left alone <span class="howmany">{{ p.left_alone.length }}</span>
        </button>
        <button :class="{ von: view === 'ruletext' }" @click="view = 'ruletext'">Rules</button>
      </nav>

      <div class="view-body">
        <div class="trees" v-if="view === 'trees'">
          <div class="side">
            <h2>Now</h2>
            <ul>
              <li v-for="e in before" :key="'b' + e.path" :class="{ shifts: e.moves, dir: e.is_dir }">
                <span class="entry"><span class="path lead">{{ lead(e.path) }}</span>{{ leaf(e.path) }}</span>
                <span class="weight" v-if="!e.is_dir">{{ bytes(e.size) }}</span>
              </li>
            </ul>
          </div>
          <div class="side">
            <h2>After tidying</h2>
            <ul>
              <li v-for="e in after" :key="'a' + e.path" :class="{ shifts: e.moves, dir: e.is_dir }">
                <span class="entry"><span class="path lead">{{ lead(e.path) }}</span>{{ leaf(e.path) }}</span>
                <span class="weight" v-if="!e.is_dir">{{ bytes(e.size) }}</span>
              </li>
            </ul>
          </div>
        </div>

        <ul class="moves" v-else-if="view === 'moves'">
          <li v-for="m in p.moves" :key="m.from">
            <span class="path">{{ m.from }}</span>
            <span class="becomes">becomes</span>
            <span class="path to">{{ m.to }}</span>
            <span class="reason">{{ m.why }}</span>
          </li>
        </ul>

        <!-- Property 4. The engine flattens eight reasons into one sentence
             before they cross the seam, so the window shows the sentence it
             wrote for each file rather than guessing at groups. Nothing is
             collapsed into a single word, which is what the property asks. -->
        <div class="alone" v-else-if="view === 'alone'">
          <p class="hint" v-if="p.left_alone.length">
            Each of these was left for a different reason.
          </p>
          <ul>
            <li v-for="a in p.left_alone" :key="a.path">
              <span class="path">{{ a.path }}</span>
              <span class="reason">{{ a.why }}</span>
            </li>
          </ul>
          <p class="hint" v-if="!p.left_alone.length">Every file here is claimed by a rule.</p>
        </div>

        <pre class="ruletext mono" v-else>{{ rules ?? "Reading the rules…" }}</pre>
      </div>

      <footer class="foot">
        <Working v-if="f.busy.value" :what="f.busy.value" note="you can leave this running" />
        <div class="act" v-else>
          <!-- Property 1: this button exists only here, beside the thing it
               would do. Property 2: the way back sits next to it at the same
               weight, and stays visible when there is nothing to undo. -->
          <Button
            look="primary"
            :disabled="!p.settles || p.tidy"
            @click="confirmTidy()"
          >Tidy up</Button>
          <Button :disabled="p.undoable === null" @click="confirmPutBack()">Put it back</Button>
          <span class="safe" v-if="p.undoable === null">
            There is nothing to put back.
          </span>
          <span class="safe" v-else>The last tidy of this folder can be undone.</span>
        </div>
      </footer>
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
.hero { display: flex; align-items: baseline; gap: var(--s3); flex-wrap: wrap; }
.big { font-size: var(--hero); font-weight: 700; letter-spacing: -0.02em; line-height: 1.1; color: var(--text); margin: 0; }
.sub { margin: 0; max-width: 52ch; }

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
:global([data-theme="retro"] .views) {
  background: none;
  padding: 0;
  gap: 4px;
  border-radius: 0;
}
:global([data-theme="retro"] .views button) {
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius) var(--radius) 0 0;
  background: var(--surface-raised);
  color: var(--text);
  padding: 5px var(--s3);
  box-shadow: 2px 0 0 var(--edge);
}
:global([data-theme="retro"] .views .von) {
  background: var(--panel);
  color: var(--text);
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
.lead { color: var(--text-faint); word-break: normal; }
.weight { font-variant-numeric: tabular-nums; flex: none; color: var(--text-faint); }

.moves li, .alone li {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0 var(--s2);
  padding: var(--s1) 0;
  font-size: var(--fine);
  color: var(--text-quiet);
  border-bottom: var(--bw) solid var(--edge);
}
.becomes { font-size: var(--fine); color: var(--text-faint); }
.to { color: var(--text); }
.reason { color: var(--text-faint); margin-left: auto; }
.hint { font-size: var(--small); color: var(--text-quiet); margin: 0 0 var(--s2); }
.ruletext {
  font-size: var(--fine);
  line-height: 1.6;
  margin: 0;
  white-space: pre-wrap;
  color: var(--text-quiet);
}

.foot {
  margin-top: auto;
  padding: var(--s3) var(--s4);
  margin-bottom: var(--s3);
  flex: none;
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius-lg);
  background: var(--panel);
  box-shadow: var(--lift-panel);
}
.act { display: flex; align-items: center; gap: var(--s3); }
.safe { font-size: var(--small); color: var(--text-faint); }
</style>
