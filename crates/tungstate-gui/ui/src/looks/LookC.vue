<!-- The workbench, round 4.
     Seed: xYxWfgJunLJWLZWKARK0febtYtFqFxpT

     Scores so far: 5, 6, 5.5. The notes that repeated across rounds are the
     ones acted on here, because a note made twice by a critic that has not
     seen its own last answer is worth more than a note made once:

       four accents (amber rail, blue rail, cream button, olive fill) -> one
       a warning callout drawn as a framework default            -> plain text
       column headers wrapping onto two lines                    -> one line
       the selected row changing height and breaking the rhythm  -> it cannot
       a wordmark competing with the one in the title bar        -> gone
       six type sizes                                            -> four

     The example move now lives above the action instead of inside the row, so
     choosing a row cannot change the height of the table. -->
<script setup lang="ts">
import { ref, computed } from "vue";
import { outcomes, refusals, learned, FOLDER, FOLDER_PATH, bytes } from "./captured";

const chosen = ref<string | null>("media");
const pickable = (o: (typeof outcomes)[number]) => o.loads && o.settles;
const picked = computed(() => outcomes.find((o) => o.name === chosen.value) ?? null);

// The folder's own rules are not a candidate: they are the situation you are
// already in. They are stated once, in a sentence, rather than given a row in
// a table of things you can choose.
const current = refusals[0];
const total = computed(() => outcomes[0]?.of ?? 0);
</script>

<template>
  <div class="c-bench">
    <div class="c-hold">
      <h1>{{ FOLDER }}</h1>
      <p class="c-where">
        <span class="c-path">{{ FOLDER_PATH }}</span>
        <button class="c-other">Point at another folder</button>
      </p>

      <p class="c-state">
        {{ learned.loose }} files sit loose at the top. The rest are one
        directory deep.
      </p>

      <h2 class="c-head">Ways you could file these {{ total }} files</h2>
      <p class="c-note" v-if="current && !current.settles">
        The rules already in this folder are not among them: they would move the
        same files every run.
      </p>

      <div class="c-table">
        <div class="c-key">
          <span class="c-kway"></span>
          <span class="c-knum">moving</span>
          <span class="c-knum">new folders</span>
          <span class="c-knum">emptied</span>
          <span class="c-knum">size</span>
        </div>

        <button
          v-for="o in outcomes"
          :key="o.name"
          class="c-row"
          :class="{ 'c-on': chosen === o.name }"
          :disabled="!pickable(o)"
          @click="chosen = o.name"
        >
          <span class="c-name">{{ o.name }}</span>
          <span class="c-said">{{ o.summary }}</span>
          <span class="c-num c-moves">{{ o.files }}</span>
          <span class="c-num">{{ o.created }}</span>
          <span class="c-num">{{ o.removed }}</span>
          <span class="c-num">{{ bytes(o.bytes) }}</span>
        </button>
      </div>

      <footer class="c-foot">
        <p class="c-eg" v-if="picked?.example">
          <span class="c-egl">for example</span>
          <span class="c-from">{{ picked.example.from }}</span>
          <span class="c-arrow">becomes</span>
          <span class="c-to">{{ picked.example.to }}</span>
        </p>
        <div class="c-act">
          <button class="c-go" :disabled="!chosen">Give this folder these rules</button>
          <span class="c-safe">You will see every move before anything happens.</span>
        </div>
      </footer>
    </div>
  </div>
</template>

<style scoped>
.c-bench {
  --ground: #3b3e37;
  --text: #eef0e9;
  --quiet: #a2a797;
  --faint: #7c8274;
  --rule: #4a4e43;
  --accent: #7fd4ff;
  --ink: #0a2230;
  --sans: "Avenir Next", ui-sans-serif, system-ui, -apple-system, sans-serif;
  --mono: ui-monospace, "SF Mono", Menlo, monospace;

  /* Four sizes, and nothing between them. */
  --display: 23px;
  --body: 14.5px;
  --small: 12.5px;
  --fine: 11.5px;

  /* One grid for the header and every row, so a figure sits under its own
     word rather than near it. `minmax(0, 1fr)` and not `1fr`: a bare `1fr`
     has an `auto` minimum, so the longest description widens column one in
     its own row only, and every row lands its figures somewhere different. */
  --cols: minmax(0, 1fr) 84px 92px 104px 88px;

  position: absolute;
  inset: 0;
  background: var(--ground);
  color: var(--text);
  font-family: var(--sans);
  font-size: var(--body);
  overflow-y: auto;
  scrollbar-gutter: stable;
}
/* A measure. Without it the descriptions end a third of the way across and
   the figures start two thirds of the way across, with nothing between. */
.c-hold {
  max-width: 860px;
  min-height: 100%;
  margin: 0 auto;
  padding: 22px 32px 0;
  display: flex;
  flex-direction: column;
}

h1 { font-size: var(--display); font-weight: 600; margin: 0; letter-spacing: -0.01em; }
.c-where { display: flex; align-items: baseline; gap: 14px; margin: 4px 0 0; min-width: 0; }
.c-path { font-family: var(--mono); font-size: var(--fine); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.c-other {
  font: inherit;
  font-size: var(--small);
  background: none;
  border: none;
  padding: 0;
  cursor: pointer;
  flex: none;
}
.c-other:hover { color: var(--text); }

.c-state { line-height: 1.55; margin: 14px 0 0; max-width: 74ch; }

.c-head {
  font-size: var(--small);
  font-weight: 600;
  margin: 18px 0 0;
  letter-spacing: 0.02em;
}
.c-table { margin-top: 10px; }
.c-note { font-size: var(--fine); margin: 3px 0 0; }

.c-key {
  display: grid;
  grid-template-columns: var(--cols);
  column-gap: 18px;
  padding: 0 12px 7px;
  margin: 0 -12px;
  border-bottom: 1px solid var(--rule);
  font-size: var(--fine);
  white-space: nowrap;
}
.c-kway { text-align: left; }
.c-knum { text-align: right; }

.c-row {
  display: grid;
  grid-template-columns: var(--cols);
  column-gap: 18px;
  align-items: center;
  font: inherit;
  text-align: left;
  background: none;
  border: none;
  color: inherit;
  /* The fill bleeds past the text on both sides, so a selected row reads as a
     band while its name still starts on the same left edge as the heading
     above it. Two left edges on one page is the fault this fixes. */
  padding: 7px 12px;
  margin: 0 -12px;
  width: calc(100% + 24px);
  border-radius: 6px;
  cursor: pointer;
}
.c-row:hover:not(:disabled):not(.c-on) { background: #41453d; }
/* One signal for one state: the row you picked is the filled one. */
.c-row.c-on { background: #4a4e44; }
.c-row:disabled { cursor: default; }

.c-name { grid-column: 1; grid-row: 1; font-weight: 600; }
.c-said { grid-column: 1; grid-row: 2; font-size: var(--small); margin-top: 1px; }
.c-num {
  grid-row: 1;
  align-self: baseline;
  text-align: right;
  font-variant-numeric: tabular-nums;
}


.c-foot {
  margin-top: auto;
  padding: 15px 12px 20px;
  margin: 0 -12px;
  border-top: 1px solid var(--rule);
  position: sticky;
  bottom: 0;
  background: var(--ground);
}
.c-eg {
  font-family: var(--mono);
  font-size: var(--fine);
  margin: 0 0 13px;
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 0 9px;
}
.c-egl { font-family: var(--sans); }
.c-from { word-break: break-all; }
.c-arrow { font-family: var(--sans); font-size: var(--small); }

.c-act { display: flex; align-items: center; gap: 16px; }
.c-go {
  font: inherit;
  font-size: var(--body);
  font-weight: 500;
  background: #e6e8e0;
  border: none;
  border-radius: 7px;
  padding: 9px 18px;
  cursor: pointer;
}
.c-go:disabled { background: #454941; cursor: default; }
.c-safe { font-size: var(--small); }

/* The stylesheet this is replacing is still loaded, and it colours elements
   this screen also uses. Rather than chase each collision, every text element
   here is given its colour outright. */
.c-bench h1, .c-bench h2, .c-bench p, .c-bench span, .c-bench button {
  color: inherit;
}
.c-bench h1 { color: var(--text); }
.c-bench .c-path { color: var(--faint); }
.c-bench .c-other { color: var(--quiet); }
.c-bench .c-state { color: var(--quiet); }
.c-bench .c-key { color: var(--quiet); }
.c-bench .c-head { color: var(--text); }
.c-bench .c-note { color: var(--quiet); }
.c-bench .c-said { color: var(--quiet); }
.c-bench .c-num { color: var(--quiet); }
.c-bench .c-moves { color: var(--text); }
.c-bench .c-row.c-on .c-num { color: var(--text); }
.c-bench .c-eg { color: var(--quiet); }
.c-bench .c-egl { color: var(--faint); }
.c-bench .c-arrow { color: var(--faint); }
.c-bench .c-to { color: var(--text); }
.c-bench .c-go { color: #24271f; }
.c-bench .c-go:disabled { color: var(--faint); }
.c-bench .c-safe { color: var(--quiet); }
</style>
