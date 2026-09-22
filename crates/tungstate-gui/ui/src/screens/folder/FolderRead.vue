<!-- Your folder, and every way you could file it, side by side.
     The screen this whole slice exists for. Nothing here has been registered,
     nothing has been written, and no file has moved. -->
<script setup lang="ts">
import { computed } from "vue";
import { useFolders } from "../../state/useFolders";
import { bytes } from "../../lib/format";
import { movedBy, made, emptied } from "../../lib/counts";
import type { Outcome } from "../../engine/types";
import Button from "../../ui/Button.vue";
import Tile from "../../ui/Tile.vue";
import Notice from "../../ui/Notice.vue";

const f = useFolders();
const chosen = defineModel<string | null>("chosen", { required: true });

/** The engine puts the folder's own rules first when it has any, with an empty
 *  summary because they are not one of the built-in layouts. They are not a
 *  candidate — they are the situation you are already in — so they come out of
 *  the list and are said in a sentence instead. */
const mine = computed(() => f.outcomes.value.find((o) => o.summary === "") ?? null);
const ways = computed(() => f.outcomes.value.filter((o) => o.summary !== ""));

const pickable = (o: Outcome) => o.loads && o.settles;
const total = computed(() => ways.value[0]?.of ?? 0);
const picked = computed(() => ways.value.find((o) => o.name === chosen.value) ?? null);

/** How the folder is filed now, in a sentence.
 *
 *  Deliberately states no total of its own. `learn_folder` counts the files
 *  its probe policy did not ignore and `compare_folder` counts every entry it
 *  walked, so the two totals differ by a `.DS_Store` or a symlink. The heading
 *  below gives the one total this screen shows; a sentence with a second one
 *  would be a contradiction a reader has to stop and work out. */
const shape = computed(() => {
  const l = f.learned.value;
  if (!l) return "";
  if (l.levels.length === 0) return "Nothing here is filed: every file sits at the top.";
  if (l.loose === 0) return `Everything here is filed by ${l.levels.join(", then ")}.`;
  return `${l.loose} files sit loose at the top. The rest are filed by ${l.levels.join(", then ")}.`;
});
</script>

<template>
  <div class="read">
    <div class="column">
      <div class="head"><Tile of="folder" :size="26" /><h1>{{ f.current.value?.name ?? f.root.value?.split("/").pop() }}</h1></div>
      <p class="locus">
        <span class="path">{{ f.root.value }}</span>
        <Button look="link" @click="f.pick()">Point at another folder</Button>
      </p>

      <p class="shape">{{ shape }}</p>

      <Notice tone="bad" v-if="f.problem.value">{{ f.problem.value }}</Notice>

      <h2 class="ways">Ways you could file these {{ total }} files</h2>

      <p class="caveat" v-if="mine && !mine.loads">
        This folder already has rules and they will not load, so they are not
        among the ways below. The file is <span class="path inline">.tungstate/policy.toml</span>.
      </p>
      <p class="caveat" v-else-if="mine && !mine.settles">
        The rules already in this folder are not among them: they would move the
        same files every run.
      </p>
      <p class="caveat" v-else-if="mine">
        This folder already has rules, so these are shown for comparison only.
        Changing them means editing
        <span class="path inline">.tungstate/policy.toml</span> yourself.
      </p>

      <div class="table">
        <div class="key">
          <span class="kway"></span>
          <span class="knum">moving</span>
          <span class="knum">new folders</span>
          <span class="knum">emptied</span>
          <span class="knum">size</span>
        </div>

        <button
          v-for="o in ways"
          :key="o.name"
          class="way"
          :class="{ on: chosen === o.name }"
          :disabled="!pickable(o)"
          @click="chosen = o.name"
        >
          <span class="wayname">{{ o.name }}</span>
          <span class="said">
            {{ o.summary }}<template v-if="!o.settles"> (these would never settle)</template>
          </span>
          <span class="num n-move">{{ movedBy(o) }}</span>
          <span class="num">{{ made(o) }}</span>
          <span class="num">{{ emptied(o) }}</span>
          <span class="num">{{ bytes(o.bytes) }}</span>
        </button>
      </div>

      <footer class="foot">
        <p class="eg" v-if="picked?.example">
          <span class="egl">for example</span>
          <span class="path">{{ picked.example.from }}</span>
          <span class="becomes">becomes</span>
          <span class="path to">{{ picked.example.to }}</span>
        </p>
        <p class="eg" v-else-if="picked && movedBy(picked) === 0">
          <span class="egl">nothing here would change</span>
        </p>
        <div class="act">
          <Button
            look="primary"
            :disabled="!chosen || !!mine"
            @click="chosen && f.choose(chosen)"
          >{{ chosen && !mine ? `Give this folder the ${chosen} rules` : "Give this folder these rules" }}</Button>
          <span class="safe">You will see every move before anything happens.</span>
        </div>
      </footer>
    </div>
  </div>
</template>

<style scoped>
.head { display: flex; align-items: center; gap: var(--s3); }
.read { position: absolute; inset: 0; overflow: hidden; }
.column {
  height: 100%;
  padding: var(--win-pad);
  padding-bottom: 0;
  display: flex;
  flex-direction: column;
}

h1 { font-size: var(--display); font-weight: 700; margin: 0; letter-spacing: -0.01em; }
.locus { display: flex; align-items: baseline; gap: var(--s4); margin: var(--s1) 0 0; min-width: 0; }
.locus .path { color: var(--text-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.shape { font-size: var(--body); line-height: 1.55; margin: var(--s4) 0 0; color: var(--text-quiet); max-width: 74ch; }

.ways { font-size: var(--small); font-weight: 600; margin: var(--s5) 0 0; }
.caveat { font-size: var(--fine); color: var(--text-quiet); margin: var(--s1) 0 0; max-width: 78ch; line-height: 1.5; }

/* One grid for the header and every row, so a figure sits under its own word
   rather than near it. `minmax(0, 1fr)` and not `1fr`: a bare `1fr` has an
   `auto` minimum, so the longest description widens column one in its own row
   alone and every row lands its figures somewhere different. */
.table { --cols: minmax(0, 1fr) 84px 92px 88px 88px; margin-top: var(--s3); padding: 0 var(--s3) var(--s4); margin-left: calc(var(--s3) * -1); margin-right: calc(var(--s3) * -1); flex: 1; min-height: 0; overflow-y: auto; }

.key {
  display: grid;
  grid-template-columns: var(--cols);
  column-gap: var(--s4);
  padding: 0 var(--s3) 7px;
  margin: 0 calc(var(--s3) * -1);
  border-bottom: 1px solid var(--edge);
  font-size: var(--fine);
  color: var(--text-faint);
  white-space: nowrap;
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--window-bg);
  padding-top: 2px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
.kway { text-align: left; }
.knum { text-align: right; }

.way {
  display: grid;
  grid-template-columns: var(--cols);
  column-gap: var(--s4);
  align-items: center;
  font: inherit;
  text-align: left;
  background: none;
  border: none;
  color: inherit;
  /* The fill bleeds past the text on both sides, so a selected row reads as a
     band while its name still starts on the same left edge as the heading
     above it. A `<button>` also shrinks to fit, and without the width every
     row is a grid of a different size. */
  padding: 7px var(--s3);
  margin: 0 calc(var(--s3) * -1);
  width: calc(100% + var(--s3) * 2);
  border-radius: var(--radius);
  cursor: pointer;
}
.way:hover:not(:disabled):not(.on) { background: var(--surface-hover); }
/* One signal for one state: the row you picked is the filled one. */
.way.on { background: var(--surface-raised); box-shadow: inset 0 0 0 1px var(--accent); }
.way:disabled { cursor: default; }

.wayname { grid-column: 1; grid-row: 1; font-weight: 600; }
.said { grid-column: 1; grid-row: 2; font-size: var(--small); color: var(--text-quiet); margin-top: 1px; }
.num { grid-row: 1; align-self: baseline; text-align: right; font-variant-numeric: tabular-nums; color: var(--text-quiet); }
.n-move { color: var(--text); }
.way.on .num { color: var(--text); }

.foot {
  margin-top: auto;
  padding: var(--s3) var(--s4);
  margin-left: calc(var(--s3) * -1);
  margin-right: calc(var(--s3) * -1);
  margin-bottom: var(--s3);
  flex: none;
  border: 1px solid var(--edge);
  border-radius: var(--radius-lg);
  background: var(--panel);
  box-shadow: var(--lift-panel);
}
.eg {
  font-size: var(--fine);
  margin: 0 0 13px;
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0 9px;
  color: var(--text-quiet);
}
.egl { color: var(--text-faint); }
.becomes { font-size: var(--small); color: var(--text-faint); }
.to { color: var(--text); }
.act { display: flex; align-items: center; gap: var(--s4); }
.safe { font-size: var(--small); color: var(--text-quiet); }
</style>
