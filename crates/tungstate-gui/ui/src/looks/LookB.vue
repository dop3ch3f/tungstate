<!-- Direction B — the report.
     Seed: VzSvLYkYxGnz7wODb4pMDdMUbJnUsLXh
     paper-white, warm · normal density · mono throughout · rounded ·
     the accent spent on what is live now.

     The thesis: this is a printed report about your folder, of the kind a
     machine used to produce on a line printer. Everything is monospaced and
     column-aligned, so the numbers line up down the page without anything
     being drawn around them. No cards, no containers, no boxes: rules and
     whitespace do all the work. -->
<script setup lang="ts">
import { ref, computed } from "vue";
import { outcomes, refusals, learned, FOLDER, FOLDER_PATH, bytes } from "./captured";

const chosen = ref<string | null>(null);
const rows = computed(() => [refusals[0], ...outcomes]);
const pickable = (o: (typeof outcomes)[number]) => o.loads && o.settles;
const pad = (s: string | number, n: number) => String(s).padStart(n, " ");
</script>

<template>
  <div class="b-page">
    <div class="b-head">
      <div class="b-tag">
        <svg class="b-mark" viewBox="0 0 32 32" aria-hidden="true">
          <rect x="1" y="1" width="30" height="30" rx="3" fill="none" stroke="currentColor" />
          <text x="16" y="21" text-anchor="middle" class="b-wo">WO</text>
        </svg>
        TUNGSTATE
      </div>
      <div class="b-meta">
        <span>read just now</span>
        <span>nothing has moved</span>
      </div>
    </div>

    <h1>{{ FOLDER }}</h1>
    <div class="b-path">{{ FOLDER_PATH }}</div>

    <div class="b-rule"></div>

    <section>
      <div class="b-k">as it is</div>
      <div class="b-v">
        <p>
          <b>{{ learned.explains }}</b> of <b>{{ learned.of }}</b> files sit in a shape
          one level deep &nbsp;·&nbsp; <b>{{ learned.loose }}</b> sit loose at the top
        </p>
        <p class="b-sug" v-for="s in learned.suggestions" :key="s.headline">
          {{ s.headline }} <span class="b-why">{{ s.why }}</span>
        </p>
      </div>
    </section>

    <div class="b-rule"></div>

    <section>
      <div class="b-k">if you filed them</div>
      <div class="b-v">
        <div class="b-cols">
          <span class="b-c1"></span>
          <span class="b-c2">moving</span>
          <span class="b-c3">made</span>
          <span class="b-c4">emptied</span>
          <span class="b-c5">size</span>
        </div>
        <button
          v-for="o in rows"
          :key="o.name"
          class="b-row"
          :class="{ 'b-on': chosen === o.name }"
          :disabled="!pickable(o)"
          @click="chosen = o.name"
        >
          <span class="b-c1">
            <span class="b-nm">{{ o.name }}</span>
            <span class="b-sm">{{ o.summary || "already in this folder" }}</span>
          </span>
          <span class="b-c2" v-if="pickable(o)">{{ pad(o.files, 2) }}/{{ o.of }}</span>
          <span class="b-c2 b-no" v-else>{{ o.loads ? "never settles" : "will not load" }}</span>
          <span class="b-c3">{{ pickable(o) ? pad(o.created, 2) : "" }}</span>
          <span class="b-c4">{{ pickable(o) ? pad(o.removed, 2) : "" }}</span>
          <span class="b-c5">{{ pickable(o) ? bytes(o.bytes) : "" }}</span>
          <span class="b-eg" v-if="o.example">
            {{ o.example.from }} <i>→</i> {{ o.example.to }}
          </span>
          <span class="b-eg b-dim" v-else>nothing would change</span>
        </button>
      </div>
    </section>

    <div class="b-rule"></div>

    <div class="b-foot">
      <span>You will see every move before anything happens.</span>
      <button class="b-go" :disabled="!chosen">
        {{ chosen ? `use ${chosen}` : "choose one" }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.b-page {
  --paper: #fdfcf9;
  --ink: #1c1a17;
  --quiet: #8a837a;
  --rule: #e6e1d8;
  --accent: #7fd4ff;
  --accent-ink: #0a2230;
  --mono: ui-monospace, "SF Mono", SFMono-Regular, Menlo, monospace;

  position: absolute;
  inset: 0;
  overflow: auto;
  background: var(--paper);
  color: var(--ink);
  font-family: var(--mono);
  font-size: 13px;
  line-height: 1.5;
  padding: 22px 46px 36px;
}

.b-head { display: flex; justify-content: space-between; align-items: center; }
.b-tag {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 11px;
  letter-spacing: 0.22em;
  color: var(--quiet);
}
.b-mark { width: 19px; height: 19px; color: var(--quiet); }
.b-wo { font-family: var(--mono); font-size: 11px; fill: currentColor; }
.b-meta { display: flex; gap: 20px; font-size: 11px; color: var(--quiet); }

h1 { font-size: 27px; font-weight: 600; margin: 26px 0 0; letter-spacing: -0.01em; }
.b-path { font-size: 12px; color: var(--quiet); margin-top: 2px; }

.b-rule { border-top: 1px solid var(--rule); margin: 20px 0; }

section { display: grid; grid-template-columns: 148px 1fr; gap: 24px; }
.b-k {
  font-size: 11px;
  letter-spacing: 0.15em;
  text-transform: uppercase;
  color: var(--quiet);
  padding-top: 2px;
}
.b-v p { margin: 0 0 6px; }
.b-v b { font-weight: 600; }
.b-sug { color: var(--quiet); }
.b-sug .b-why { display: block; font-size: 11.5px; opacity: 0.75; }

.b-cols,
.b-row {
  display: grid;
  grid-template-columns: 1fr 74px 52px 62px 78px;
  gap: 10px;
  align-items: baseline;
  text-align: right;
  width: 100%;
}
.b-cols {
  font-size: 10.5px;
  letter-spacing: 0.1em;
  text-transform: uppercase;
  color: var(--quiet);
  padding-bottom: 7px;
  border-bottom: 1px solid var(--rule);
}
.b-cols .b-c1 { text-align: left; }

.b-row {
  background: none;
  border: none;
  border-bottom: 1px solid var(--rule);
  font: inherit;
  color: inherit;
  padding: 9px 0;
  cursor: pointer;
}
.b-row:hover:not(:disabled) { background: #f4f1ea; }
.b-row.b-on { background: color-mix(in srgb, var(--accent) 24%, transparent); }
.b-row:disabled { cursor: default; color: var(--quiet); }

.b-c1 { text-align: left; }
.b-nm { display: block; font-weight: 600; }
.b-sm { display: block; font-size: 11.5px; color: var(--quiet); }
.b-c2 { font-variant-numeric: tabular-nums; }
.b-c2.b-no { font-size: 11.5px; }
.b-c3,
.b-c4,
.b-c5 { font-variant-numeric: tabular-nums; font-size: 12px; color: var(--quiet); }

.b-eg {
  grid-column: 1 / -1;
  text-align: left;
  font-size: 11.5px;
  color: var(--quiet);
  margin-top: 4px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.b-eg i { font-style: normal; color: var(--ink); padding: 0 5px; }
.b-eg.b-dim { opacity: 0.6; }

.b-foot { display: flex; justify-content: space-between; align-items: center; color: var(--quiet); }
.b-go {
  font-family: var(--mono);
  font-size: 13px;
  background: var(--accent);
  color: var(--accent-ink);
  border: none;
  border-radius: 5px;
  padding: 9px 20px;
  cursor: pointer;
}
.b-go:disabled { background: none; border: 1px solid var(--rule); color: var(--quiet); cursor: default; }

/* See LookA: the stylesheet being replaced is still loaded. */
.b-page h1, .b-page p, .b-page span, .b-page b, .b-page i, .b-page button {
  color: inherit;
}
.b-page h1 { color: var(--ink); }
/* Restated after the blanket rule above, which outranks a single class. */
.b-page .b-go { color: var(--accent-ink); }
.b-page .b-go:disabled { color: var(--quiet); }
</style>
