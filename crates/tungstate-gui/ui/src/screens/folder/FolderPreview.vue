<!-- What would happen, before anything does; then the two buttons.
     Safety properties 1 to 5 all live on this screen. -->
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useFolders } from "../../state/useFolders";
import { folders } from "../../engine/commands";
import { bytes, duration } from "../../lib/format";
import { wouldMove, outOf, moved, skipped, failed, putBack, files, dirsCounted } from "../../lib/counts";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import Working from "../../ui/Working.vue";

const f = useFolders();
type View = "trees" | "moves" | "alone" | "ruletext";
const view = ref<View>("trees");
const rules = ref<string | null>(null);

const p = computed(() => f.preview.value);

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

async function confirmTidy() {
  const view = p.value;
  if (!view) return;
  const detail = [
    `${files(wouldMove(view))} would move, of ${outOf(view)}.`,
    `${bytes(view.bytes)} in all.`,
    `${dirs.value.made} directories made, ${dirs.value.emptied} emptied.`,
  ];
  const answer = await ask({
    title: `Tidy ${view.folder}?`,
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
      <!-- The folder's own name, not `p.folder`, which is the name the policy
           file declares. A folder called `messy-downloads` governed by the
           `downloads` layout was rendering as "downloads". -->
      <h1>{{ f.current.value?.name ?? f.root.value?.split("/").pop() }}</h1>
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
            {{ files(skipped(f.tidied.value)) }} left alone: they changed while we looked.
          </template>
          <template v-if="f.tidied.value.failed">
            {{ files(failed(f.tidied.value)) }} could not be moved.
          </template>
        </template>
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
      <p class="sum" v-else>
        <b>{{ files(wouldMove(p)) }}</b> of {{ outOf(p) }} would move, {{ bytes(p.bytes) }}.
        It would make {{ dirs.made }} director{{ dirs.made === 1 ? "y" : "ies" }}
        and empty {{ dirs.emptied }}.
      </p>

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
              <li v-for="e in p.before" :key="'b' + e.path" :class="{ shifts: e.moves }">
                <span class="path">{{ e.path }}</span>
                <span class="weight" v-if="!e.is_dir">{{ bytes(e.size) }}</span>
              </li>
            </ul>
          </div>
          <div class="side">
            <h2>After tidying</h2>
            <ul>
              <li v-for="e in p.after" :key="'a' + e.path" :class="{ shifts: e.moves }">
                <span class="path">{{ e.path }}</span>
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
.prev { position: absolute; inset: 0; overflow-y: auto; scrollbar-gutter: stable; }
.column {
  max-width: 980px;
  min-height: 100%;
  margin: 0 auto;
  padding: var(--s5) var(--s6) 0;
  display: flex;
  flex-direction: column;
}
h1 { font-size: var(--display); font-weight: 600; margin: 0; letter-spacing: -0.01em; }
.locus { display: flex; gap: var(--s3); align-items: baseline; margin: var(--s1) 0 0; }
.ruleset { font-size: var(--fine); color: var(--text-faint); flex: none; }
.locus .path { color: var(--text-faint); }
.did { margin-top: var(--s3); }

.sum { font-size: var(--body); line-height: 1.55; margin: var(--s4) 0 0; color: var(--text-quiet); }
.sum b { color: var(--text); font-weight: 600; }

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
.howmany { font-variant-numeric: tabular-nums; opacity: 0.7; margin-left: 2px; }

.view-body { margin-top: var(--s3); flex: 1; padding-bottom: var(--s4); }

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
/* Exactly one accent on this screen, spent on the files that would change. */
.side li.shifts { color: var(--accent); }
.weight { font-variant-numeric: tabular-nums; flex: none; color: var(--text-faint); }

.moves li, .alone li {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0 var(--s2);
  padding: var(--s1) 0;
  font-size: var(--fine);
  color: var(--text-quiet);
  border-bottom: 1px solid var(--edge);
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
  padding: var(--s4) 0 20px;
  border-top: 1px solid var(--edge);
  position: sticky;
  bottom: 0;
  background: var(--surface);
}
.act { display: flex; align-items: center; gap: var(--s3); }
.safe { font-size: var(--small); color: var(--text-faint); }
</style>
