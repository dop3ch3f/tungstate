<!-- The groups, each one opening into its copies. Every copy has its own
     checkbox, because keeping two of four is a thing people want. -->
<script setup lang="ts">
import { useDupes } from "../../state/useDupes";
import { bytes, shortPath, when } from "../../lib/format";
import { picture } from "../../engine/pictures";
import type { DupeCopy, DupeRow } from "../../engine/types";
import Empty from "../../ui/Empty.vue";

const d = useDupes();

// A lookup, because the CSS checker cannot follow a computed class name.
const DOT: Record<DupeRow["kind"], string> = {
  pictures: "k-image",
  video: "k-video",
  sound: "k-audio",
  documents: "k-doc",
  archives: "k-archive",
  applications: "k-archive",
  folders: "k-folder",
  other: "k-archive",
};

function dated(copy: DupeCopy): string {
  return copy.mtime ? when(Date.parse(copy.mtime)) : "no date";
}

function lead(row: DupeRow): DupeCopy {
  return row.copies[0];
}

function toggle(row: DupeRow) {
  d.open(row.id, !d.opened.value.has(row.id));
  d.highlighted.value = lead(row).path;
}
</script>

<template>
  <div class="dl">
    <Empty
      v-if="!d.shown.value.length"
      art="no-folders"
      line="Nothing of this kind was found here."
    />

    <div v-for="row in d.shown.value" :key="row.id" class="group">
      <button class="head" :aria-expanded="d.opened.value.has(row.id)" @click="toggle(row)">
        <span class="twist" aria-hidden="true">›</span>
        <img
          v-if="lead(row).thumb"
          class="thumb"
          :src="picture(lead(row).thumb!)"
          alt=""
        />
        <span v-else class="thumb blank" :class="DOT[row.kind]" aria-hidden="true"></span>
        <span class="about">
          <span class="name">{{ lead(row).name }}</span>
          <span class="note">
            {{ bytes(row.reclaimable) }} to reclaim
            <template v-if="row.folder">· {{ row.files }} file(s) each</template>
            <template v-if="!row.sure">· matched on samples</template>
          </span>
        </span>
        <span class="count">{{ d.tickedIn(row) }} <i>of</i> {{ row.copies.length }}</span>
      </button>

      <ul v-if="d.opened.value.has(row.id)" class="copies">
        <li
          v-for="copy in row.copies"
          :key="copy.path"
          :aria-current="d.showing.value?.copy.path === copy.path ? 'true' : undefined"
        >
          <label class="box">
            <input
              type="checkbox"
              :checked="d.ticked.value.has(copy.path)"
              @change="d.tick(copy.path, ($event.target as HTMLInputElement).checked)"
            />
          </label>
          <button class="copy" @click="d.highlighted.value = copy.path">
            <span class="path">{{ shortPath(copy.path, 4) }}</span>
            <span class="facts">
              {{ bytes(copy.size) }} · {{ dated(copy) }}
              <template v-if="copy.keep">· the one it would keep</template>
              <template v-else-if="copy.alike !== null">· {{ copy.alike }}% alike</template>
            </span>
          </button>
        </li>
      </ul>
    </div>
  </div>
</template>

<style scoped>
.dl { overflow-y: auto; padding-right: var(--s2); }
.group { border-bottom: var(--bw) solid var(--edge); }
.head {
  display: flex;
  align-items: center;
  gap: var(--s2);
  width: 100%;
  padding: var(--s2);
  font: inherit;
  text-align: left;
  color: var(--text);
  background: none;
  border: none;
  cursor: pointer;
}
.head:hover { background: var(--surface-hover); }
.twist {
  width: 12px;
  flex: none;
  color: var(--text-faint);
  text-align: center;
  transition: transform var(--quick) var(--ease);
}
.head[aria-expanded="true"] .twist { transform: rotate(90deg); }
.thumb {
  width: 34px;
  height: 34px;
  flex: none;
  object-fit: cover;
  border-radius: var(--radius);
  background: var(--surface-raised);
}
.blank { display: block; opacity: 0.22; }
.k-image { background: var(--kind-image); }
.k-video { background: var(--kind-video); }
.k-audio { background: var(--kind-audio); }
.k-doc { background: var(--kind-doc); }
.k-archive { background: var(--kind-archive); }
.k-folder { background: var(--kind-folder); }
.about { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
.name { font-size: var(--small); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.note { font-size: var(--fine); color: var(--text-faint); }
.count { font-size: var(--fine); color: var(--text-quiet); font-variant-numeric: tabular-nums; flex: none; }
.count i { color: var(--text-faint); font-style: normal; }
.copies { list-style: none; margin: 0; padding: 0 0 var(--s2); }
.copies li { display: flex; align-items: center; gap: var(--s2); padding-left: var(--s4); }
.copies li[aria-current="true"] { background: var(--surface-raised); }
.box { display: flex; align-items: center; padding: 0 var(--s1); }
.copy {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 4px var(--s2) 4px 0;
  font: inherit;
  text-align: left;
  color: var(--text-quiet);
  background: none;
  border: none;
  cursor: pointer;
}
.path { font-size: var(--small); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.facts { font-size: var(--fine); color: var(--text-faint); }
</style>
