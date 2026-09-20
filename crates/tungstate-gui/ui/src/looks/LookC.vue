<!-- Direction C — the workbench.
     Seed: xYxWfgJunLJWLZWKARK0febtYtFqFxpT
     mid-grey, warm-green · airy · humanist + mono · rounded ·
     the accent spent on what would change.

     The thesis: a work surface rather than a document. Each way of filing is a
     tile you can pick up, and every tile carries a bar of the folder itself —
     the blue part is what that layout would move, the grey part is what it
     would leave. The numbers are still there, but the shape of the answer
     arrives before you read any of them. -->
<script setup lang="ts">
import { ref, computed } from "vue";
import { outcomes, refusals, learned, FOLDER, FOLDER_PATH, bytes } from "./captured";

const chosen = ref<string | null>("media");
const rows = computed(() => [refusals[0], ...outcomes]);
const pickable = (o: (typeof outcomes)[number]) => o.loads && o.settles;
const share = (o: (typeof outcomes)[number]) => Math.round((o.files / o.of) * 100);
const picked = computed(() => rows.value.find((r) => r.name === chosen.value) ?? null);
</script>

<template>
  <div class="c-bench">
    <aside>
      <div class="c-brand">
        <svg class="c-mark" viewBox="0 0 32 32" aria-hidden="true">
          <rect x="1" y="1" width="30" height="30" rx="5" fill="none" stroke="currentColor" />
          <text x="16" y="21" text-anchor="middle" class="c-wo">WO</text>
        </svg>
      </div>
      <h1>{{ FOLDER }}</h1>
      <p class="c-path">{{ FOLDER_PATH }}</p>

      <div class="c-asis">
        <p class="c-big">
          <b>{{ learned.loose }}</b> of {{ learned.of }} files sit loose at the top
        </p>
        <p class="c-sub">
          The {{ learned.explains }} that do not are one directory deep. There is
          little shape here to keep.
        </p>
      </div>

      <ul class="c-sug">
        <li v-for="s in learned.suggestions" :key="s.headline">{{ s.headline }}</li>
      </ul>

      <div class="c-spacer"></div>

      <div class="c-chose" v-if="picked && pickable(picked)">
        <span class="c-lbl">for instance</span>
        <p class="c-from">{{ picked.example?.from }}</p>
        <p class="c-to">→ {{ picked.example?.to }}</p>
      </div>
      <button class="c-go" :disabled="!chosen">Give this folder these rules</button>
      <p class="c-safe">Nothing moves until you have seen all of it.</p>
    </aside>

    <main>
      <h2>Ways you could file them</h2>
      <div class="c-grid">
        <button
          v-for="o in rows"
          :key="o.name"
          class="c-tile"
          :class="{ 'c-on': chosen === o.name, 'c-off': !pickable(o) }"
          :disabled="!pickable(o)"
          @click="chosen = o.name"
        >
          <span class="c-nm">{{ o.name }}</span>
          <span class="c-sm">{{ o.summary || "the rules already here" }}</span>

          <template v-if="pickable(o)">
            <span class="c-bar" :title="`${o.files} of ${o.of}`">
              <i :style="{ width: share(o) + '%' }"></i>
            </span>
            <span class="c-num"><b>{{ o.files }}</b> of {{ o.of }} would move</span>
            <span class="c-dirs">
              <span>{{ o.created }} made</span>
              <span>{{ o.removed }} emptied</span>
              <span>{{ bytes(o.bytes) }}</span>
            </span>
          </template>
          <template v-else>
            <span class="c-refuse">{{ o.loads ? "never settles" : "will not load" }}</span>
            <span class="c-why">
              {{ o.loads
                ? "these rules would keep moving the same files for ever"
                : "the file has a mistake in it, on line 7" }}
            </span>
          </template>
        </button>
      </div>
    </main>
  </div>
</template>

<style scoped>
.c-bench {
  --ground: #3b3e37;
  --raised: #474b43;
  --lift: #515649;
  --text: #eef0e9;
  --quiet: #a7ac9c;
  --rule: #555a4d;
  --accent: #7fd4ff;
  --accent-ink: #0a2230;
  --sans: "Avenir Next", ui-sans-serif, system-ui, -apple-system, sans-serif;
  --mono: ui-monospace, "SF Mono", Menlo, monospace;

  position: absolute;
  inset: 0;
  display: grid;
  grid-template-columns: 310px 1fr;
  background: var(--ground);
  color: var(--text);
  font-family: var(--sans);
  overflow: hidden;
}

aside {
  display: flex;
  flex-direction: column;
  padding: 24px 26px 26px;
  border-right: 1px solid var(--rule);
  overflow: auto;
}
.c-brand { color: var(--quiet); margin-bottom: 26px; }
.c-mark { width: 24px; height: 24px; }
.c-wo { font-family: var(--sans); font-size: 12px; font-weight: 600; fill: currentColor; }

h1 { font-size: 24px; font-weight: 600; margin: 0; letter-spacing: -0.01em; }
.c-path { font-family: var(--mono); font-size: 11.5px; color: var(--quiet); margin: 3px 0 0; }

.c-asis { margin-top: 26px; }
.c-big { font-size: 16px; line-height: 1.5; margin: 0; }
.c-big b { font-size: 24px; font-weight: 600; }
.c-sub { font-size: 13px; color: var(--quiet); line-height: 1.55; margin: 8px 0 0; }

.c-sug { list-style: none; margin: 20px 0 0; padding: 0; }
.c-sug li {
  font-size: 13px;
  color: var(--quiet);
  line-height: 1.5;
  padding: 7px 0;
  border-top: 1px solid var(--rule);
}
.c-spacer { flex: 1; min-height: 20px; }

.c-chose { margin-bottom: 16px; }
.c-lbl {
  font-size: 10.5px;
  letter-spacing: 0.13em;
  text-transform: uppercase;
  color: var(--quiet);
}
.c-from,
.c-to {
  font-family: var(--mono);
  font-size: 11.5px;
  margin: 5px 0 0;
  word-break: break-all;
}
.c-from { color: var(--quiet); }
.c-bench .c-to { color: var(--accent); }

.c-go {
  font: inherit;
  font-size: 14px;
  background: var(--accent);
  color: var(--accent-ink);
  border: none;
  border-radius: 8px;
  padding: 12px 16px;
  cursor: pointer;
  width: 100%;
}
.c-go:disabled { background: var(--raised); color: var(--quiet); cursor: default; }
.c-safe { font-size: 11.5px; color: var(--quiet); text-align: center; margin: 10px 0 0; }

main { padding: 26px 28px; overflow: auto; }
h2 {
  font-size: 11.5px;
  letter-spacing: 0.13em;
  text-transform: uppercase;
  color: var(--quiet);
  font-weight: 500;
  margin: 0 0 16px;
}
.c-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(212px, 1fr));
  gap: 12px;
}

.c-tile {
  font: inherit;
  text-align: left;
  background: var(--raised);
  border: 1px solid transparent;
  border-radius: 10px;
  padding: 15px 16px 14px;
  color: inherit;
  cursor: pointer;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.c-tile:hover:not(:disabled) { background: var(--lift); }
.c-tile.c-on { border-color: var(--accent); background: var(--lift); }
.c-tile.c-off { cursor: default; opacity: 0.72; }

.c-nm { font-size: 15px; font-weight: 600; }
.c-sm { font-size: 12px; color: var(--quiet); line-height: 1.4; min-height: 34px; }

.c-bar {
  display: block;
  height: 6px;
  border-radius: 3px;
  background: #2f322c;
  overflow: hidden;
  margin: 6px 0 9px;
}
.c-bar i { display: block; height: 100%; background: var(--accent); }

.c-num { font-size: 13px; }
.c-num b { font-size: 17px; font-weight: 600; }
.c-dirs {
  display: flex;
  gap: 11px;
  font-size: 11px;
  color: var(--quiet);
  margin-top: 5px;
  font-variant-numeric: tabular-nums;
}

.c-refuse { font-size: 13px; color: #e2b05f; margin-top: 12px; }
.c-why { font-size: 11.5px; color: var(--quiet); line-height: 1.45; margin-top: 3px; }

/* The stylesheet this is replacing is still loaded, and it colours elements
   this screen also uses. Rather than chase each collision, every text element
   here is given its colour outright. */
.c-bench h1, .c-bench h2, .c-bench p, .c-bench li, .c-bench span,
.c-bench b, .c-bench i, .c-bench button {
  color: inherit;
}
.c-bench h1 { color: var(--text); }
/* Restated after the blanket rule above, which outranks a single class. */
.c-bench .c-go { color: var(--accent-ink); }
.c-bench .c-go:disabled { color: var(--quiet); }
.c-bench .c-to { color: var(--accent); }
.c-bench .c-refuse { color: #e2b05f; }
</style>
