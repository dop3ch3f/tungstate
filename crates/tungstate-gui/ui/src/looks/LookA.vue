<!-- Direction A — the measuring instrument.
     Seed: O78kyuj9pqQf6spnR7sVw7PYaR3AXpIX
     ink-on-cream · airy · serif text + grotesque figures · rounded ·
     the accent spent on the thing you have chosen.

     The thesis: a folder is a measurement, and this is its spec sheet. One
     column, one row per way of filing, the figures set large and tabular so
     the eye compares down the column instead of reading each row. -->
<script setup lang="ts">
import { ref, computed } from "vue";
import { outcomes, refusals, learned, FOLDER, FOLDER_PATH, bytes } from "./captured";

const chosen = ref<string | null>("downloads");
const rows = computed(() => [refusals[0], ...outcomes]);
const pickable = (o: (typeof outcomes)[number]) => o.loads && o.settles;
</script>

<template>
  <div class="a-sheet">
    <header>
      <div class="a-ident">
        <svg class="a-mark" viewBox="0 0 32 32" aria-hidden="true">
          <rect x="1" y="1" width="30" height="30" rx="4" fill="none" stroke="currentColor" />
          <text x="16" y="21" text-anchor="middle" class="a-wo">WO</text>
        </svg>
        <span>tungstate</span>
      </div>
      <button class="a-ghost">Point at another folder</button>
    </header>

    <div class="a-lede">
      <h1>{{ FOLDER }}</h1>
      <p class="a-where">{{ FOLDER_PATH }}</p>
      <p class="a-read">
        Thirty files. Twenty-eight of them sit loose at the top, and the two that
        do not are only one directory deep. There is not much shape here to keep.
      </p>
      <ul class="a-notes">
        <li v-for="s in learned.suggestions" :key="s.headline">{{ s.headline }}</li>
      </ul>
    </div>

    <table class="a-ways">
      <caption>What each way of filing would do to these files</caption>
      <tbody>
        <tr
          v-for="o in rows"
          :key="o.name"
          :class="{ 'a-picked': chosen === o.name, 'a-refused': !pickable(o) }"
          @click="pickable(o) && (chosen = o.name)"
        >
          <td class="a-which">
            <span class="a-nm">{{ o.name }}</span>
            <span class="a-sm">{{ o.summary || "the rules already in this folder" }}</span>
          </td>
          <td class="a-fig">
            <template v-if="pickable(o)">
              <b>{{ o.files }}</b><span class="a-of">of {{ o.of }}</span>
            </template>
            <em v-else-if="!o.loads">will not load</em>
            <em v-else>never settles</em>
          </td>
          <td class="a-fig a-sub"><b>{{ o.created }}</b><span class="a-of">made</span></td>
          <td class="a-fig a-sub"><b>{{ o.removed }}</b><span class="a-of">emptied</span></td>
          <td class="a-size">{{ pickable(o) ? bytes(o.bytes) : "—" }}</td>
        </tr>
      </tbody>
    </table>

    <div class="a-eg" v-if="chosen">
      <span class="a-lbl">For instance</span>
      <p>
        <code>{{ rows.find((r) => r.name === chosen)?.example?.from }}</code>
        <span class="a-arrow">becomes</span>
        <code>{{ rows.find((r) => r.name === chosen)?.example?.to }}</code>
      </p>
    </div>

    <footer>
      <span class="a-hint">Nothing has moved. You will see the whole change before anything does.</span>
      <button class="a-go" :disabled="!chosen">Give this folder these rules</button>
    </footer>
  </div>
</template>

<style scoped>
.a-sheet {
  --ground: #f4f0ee;
  --raised: #fbf9f8;
  --ink: #22201f;
  --quiet: #6f6763;
  --rule: #e0d8d3;
  --accent: #7fd4ff;
  --accent-ink: #0a2230;
  --serif: ui-serif, "Iowan Old Style", Georgia, serif;
  --grot: ui-sans-serif, system-ui, -apple-system, sans-serif;

  position: absolute;
  inset: 0;
  overflow: auto;
  background: var(--ground);
  color: var(--ink);
  font-family: var(--serif);
  padding: 0 64px 40px;
}

header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 20px 0 36px;
}
.a-ident {
  display: flex;
  align-items: center;
  gap: 9px;
  font-family: var(--grot);
  font-size: 12px;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: var(--quiet);
}
.a-mark { width: 22px; height: 22px; color: var(--quiet); }
.a-wo { font-family: var(--grot); font-size: 12px; font-weight: 600; fill: currentColor; }
.a-ghost {
  font-family: var(--grot);
  font-size: 12.5px;
  background: none;
  border: 1px solid var(--rule);
  border-radius: 999px;
  padding: 6px 15px;
  color: var(--quiet);
  cursor: pointer;
}
.a-ghost:hover { border-color: var(--quiet); color: var(--ink); }

h1 { font-size: 40px; font-weight: 400; margin: 0; letter-spacing: -0.015em; }
.a-lede { max-width: 72ch; }

.a-where {
  font-family: var(--grot);
  font-size: 12px;
  color: var(--quiet);
  margin: 4px 0 0;
}
.a-read { font-size: 17px; line-height: 1.6; max-width: 54ch; margin: 22px 0 0; }
.a-notes { margin: 14px 0 0; padding: 0; list-style: none; }
.a-notes li {
  font-size: 15px;
  color: var(--quiet);
  padding-left: 18px;
  position: relative;
  line-height: 1.7;
}
.a-notes li::before { content: "·"; position: absolute; left: 6px; }

.a-ways { width: 100%; border-collapse: collapse; margin-top: 44px; }
caption {
  text-align: left;
  font-family: var(--grot);
  font-size: 11.5px;
  letter-spacing: 0.13em;
  text-transform: uppercase;
  color: var(--quiet);
  padding-bottom: 10px;
}
.a-ways tr {
  border-top: 1px solid var(--rule);
  cursor: pointer;
}
.a-ways tr:last-child { border-bottom: 1px solid var(--rule); }
.a-ways td { padding: 15px 0; vertical-align: baseline; }
.a-ways tr.a-picked { background: color-mix(in srgb, var(--accent) 22%, transparent); }
.a-ways tr.a-refused { cursor: default; color: var(--quiet); }

.a-which { width: 46%; }
.a-nm { display: block; font-size: 19px; }
.a-sm {
  display: block;
  font-family: var(--grot);
  font-size: 12.5px;
  color: var(--quiet);
  margin-top: 2px;
}

.a-fig { font-family: var(--grot); text-align: right; white-space: nowrap; padding-right: 26px !important; }
.a-fig b { font-size: 25px; font-weight: 500; font-variant-numeric: tabular-nums; }
.a-fig .a-of { font-size: 11.5px; color: var(--quiet); display: block; margin-top: 1px; }
.a-fig.a-sub b { font-size: 17px; color: var(--quiet); }
.a-fig em { font-family: var(--grot); font-size: 13px; font-style: normal; }
.a-size {
  font-family: var(--grot);
  font-size: 13px;
  color: var(--quiet);
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.a-eg { margin-top: 26px; }
.a-lbl {
  font-family: var(--grot);
  font-size: 11.5px;
  letter-spacing: 0.13em;
  text-transform: uppercase;
  color: var(--quiet);
}
.a-eg p { margin: 8px 0 0; display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
.a-eg code {
  font-family: ui-monospace, "SF Mono", Menlo, monospace;
  font-size: 12.5px;
  background: var(--raised);
  border: 1px solid var(--rule);
  border-radius: 4px;
  padding: 4px 8px;
}
.a-arrow { font-size: 14px; color: var(--quiet); }

footer {
  margin-top: 40px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 24px;
}
.a-hint { font-size: 14px; color: var(--quiet); }
.a-go {
  font-family: var(--grot);
  font-size: 14px;
  background: var(--accent);
  color: var(--accent-ink);
  border: none;
  border-radius: 999px;
  padding: 11px 26px;
  cursor: pointer;
}
.a-go:disabled { opacity: 0.4; cursor: default; }

/* The old global stylesheet is still loaded while the directions are being
   judged, and it colours elements this mockup also uses. Rather than chase
   each collision, every text element here is given its colour outright. */
.a-sheet h1, .a-sheet p, .a-sheet li, .a-sheet td, .a-sheet caption,
.a-sheet span, .a-sheet b, .a-sheet em, .a-sheet code, .a-sheet button {
  color: inherit;
}
.a-sheet h1 { color: var(--ink); }
.a-sheet .a-read { color: var(--ink); }
/* Restated after the blanket rule above, which outranks a single class. */
.a-sheet .a-go { color: var(--accent-ink); }
.a-sheet .a-ghost { color: var(--quiet); }
</style>
