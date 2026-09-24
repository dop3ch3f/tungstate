<!-- What kind of thing, and how much of it. The first screen in the app that
     answers "where did my disk go" before it answers "what can I delete". -->
<script setup lang="ts">
import { computed } from "vue";
import { useDupes } from "../../state/useDupes";
import { bytes } from "../../lib/format";
import { files } from "../../lib/counts";
import type { Claim, DupeKind } from "../../engine/types";

const d = useDupes();

const SECTIONS: { claim: Claim; title: string }[] = [
  { claim: "identical", title: "Exact duplicates" },
  { claim: "same", title: "The same, re-wrapped" },
  { claim: "similar", title: "Looks alike" },
];

const KINDS: { kind: DupeKind; label: string }[] = [
  { kind: "pictures", label: "Pictures" },
  { kind: "video", label: "Video" },
  { kind: "sound", label: "Sound" },
  { kind: "documents", label: "Documents" },
  { kind: "archives", label: "Archives" },
  { kind: "applications", label: "Applications" },
  { kind: "folders", label: "Folders" },
  { kind: "other", label: "Everything else" },
];

// A lookup, because the CSS checker cannot follow a computed class name.
const DOT: Record<DupeKind, string> = {
  pictures: "k-image",
  video: "k-video",
  sound: "k-audio",
  documents: "k-doc",
  archives: "k-archive",
  applications: "k-archive",
  folders: "k-folder",
  other: "k-archive",
};

const sections = computed(() =>
  SECTIONS.filter((section) => d.tallies.value[section.claim])
    .map((section) => ({
      ...section,
      total: d.tallies.value[section.claim],
      kinds: KINDS.map((kind) => ({
        ...kind,
        total: d.tallies.value[`${section.claim}:${kind.kind}`],
      })).filter((kind) => kind.total),
    })),
);

function open(claim: Claim, kind: DupeKind | null) {
  d.drawer.value = { claim, kind };
  d.highlighted.value = null;
}

function isOpen(claim: Claim, kind: DupeKind | null): boolean {
  return d.drawer.value.claim === claim && d.drawer.value.kind === kind;
}
</script>

<template>
  <nav class="dw" aria-label="What was found">
    <section v-for="section in sections" :key="section.claim">
      <h2>{{ section.title }}</h2>
      <button
        class="row all"
        :aria-current="isOpen(section.claim, null) ? 'true' : undefined"
        @click="open(section.claim, null)"
      >
        <span class="name">All of them</span>
        <span class="size">{{ bytes(section.total.bytes) }}</span>
      </button>
      <button
        v-for="kind in section.kinds"
        :key="kind.kind"
        class="row"
        :aria-current="isOpen(section.claim, kind.kind) ? 'true' : undefined"
        @click="open(section.claim, kind.kind)"
      >
        <span class="dot" :class="DOT[kind.kind]"></span>
        <span class="name">{{ kind.label }}</span>
        <span class="size">{{ bytes(kind.total.bytes) }}</span>
      </button>
    </section>

    <section class="chosen">
      <h2>Ticked</h2>
      <p class="tick-total">{{ bytes(d.chosenBytes.value) }}</p>
      <p class="tick-count">in {{ files(d.chosenFiles.value) }}</p>
    </section>
  </nav>
</template>

<style scoped>
.dw {
  display: flex;
  flex-direction: column;
  gap: var(--s4);
  overflow-y: auto;
  padding-right: var(--s2);
}
h2 {
  font-size: var(--fine);
  font-weight: 700;
  color: var(--text-faint);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  margin: 0 0 var(--s1);
}
.row {
  display: flex;
  align-items: center;
  gap: var(--s2);
  width: 100%;
  padding: 5px var(--s2);
  margin-bottom: 1px;
  font: inherit;
  font-size: var(--small);
  color: var(--text-quiet);
  text-align: left;
  background: none;
  border: none;
  border-radius: var(--radius);
  cursor: pointer;
}
.row:hover { color: var(--text); background: var(--surface-hover); }
.row[aria-current="true"] { color: var(--text); background: var(--surface-raised); font-weight: 600; }
.all { font-weight: 600; }
.name { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.size { font-size: var(--fine); color: var(--text-faint); font-variant-numeric: tabular-nums; }
.dot { width: 7px; height: 7px; border-radius: 2px; flex: none; }
.k-image { background: var(--kind-image); }
.k-video { background: var(--kind-video); }
.k-audio { background: var(--kind-audio); }
.k-doc { background: var(--kind-doc); }
.k-archive { background: var(--kind-archive); }
.k-folder { background: var(--kind-folder); }
.chosen { margin-top: auto; padding-top: var(--s3); border-top: var(--bw) solid var(--edge); }
.tick-total { font-size: var(--title); font-weight: 700; margin: 0; letter-spacing: -0.01em; }
.tick-count { font-size: var(--fine); color: var(--text-faint); margin: 2px 0 0; }
</style>
