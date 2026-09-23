<!-- The file itself, as large as the pane allows. Nobody deletes a photograph
     they cannot see, which is the whole reason this pane exists. -->
<script setup lang="ts">
import { computed } from "vue";
import { useDupes } from "../../state/useDupes";
import { bytes, shortPath, when } from "../../lib/format";
import { picture } from "../../engine/pictures";
import Pixels from "../../ui/Pixels.vue";

const d = useDupes();
const at = computed(() => d.showing.value);
</script>

<template>
  <aside class="dp" v-if="at">
    <header>
      <h2>{{ at.copy.name }}</h2>
      <p class="tally">
        {{ d.tickedIn(at.row) }} of {{ at.row.copies.length }} ticked
      </p>
    </header>

    <div class="frame">
      <img v-if="at.copy.thumb" :src="picture(at.copy.thumb)" :alt="at.copy.name" />
      <div v-else class="none">
        <Pixels of="dupes" :size="48" />
        <p>No picture of this kind of file.</p>
      </div>
    </div>

    <dl>
      <dt>Where</dt>
      <dd>{{ shortPath(at.copy.path, 6) }}</dd>
      <dt>Size</dt>
      <dd>{{ bytes(at.copy.size) }}</dd>
      <dt>Written</dt>
      <dd>{{ at.copy.mtime ? when(Date.parse(at.copy.mtime)) : "no date" }}</dd>
      <template v-if="at.copy.alike !== null">
        <dt>Alike</dt>
        <dd>{{ at.copy.alike }}% like the copy above it</dd>
      </template>
    </dl>

    <p class="claim" v-if="at.row.claim === 'identical'">
      Byte for byte the same file. This is proof, not a guess.
    </p>
    <p class="claim" v-else-if="at.row.claim === 'same'">
      The same thing in a different wrapper: a re-export, a resize, or a second
      save at another quality.
    </p>
    <p class="claim" v-else>
      These only look alike. Two photographs of one moment are not one
      photograph, so look before you tick.
    </p>
  </aside>
</template>

<style scoped>
.dp {
  display: flex;
  flex-direction: column;
  gap: var(--s3);
  overflow-y: auto;
  padding-left: var(--s4);
  border-left: var(--bw) solid var(--edge);
}
h2 { font-size: var(--body); font-weight: 700; margin: 0; overflow-wrap: anywhere; }
.tally { font-size: var(--fine); color: var(--text-faint); margin: 2px 0 0; }
.frame {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 160px;
  background: var(--surface-raised);
  border-radius: var(--radius-lg);
  overflow: hidden;
}
.frame img { max-width: 100%; max-height: 320px; display: block; }
.none { display: flex; flex-direction: column; align-items: center; gap: var(--s2); padding: var(--s4); }
.none p { font-size: var(--fine); color: var(--text-faint); margin: 0; text-align: center; }
dl { display: grid; grid-template-columns: auto 1fr; gap: 3px var(--s3); margin: 0; font-size: var(--small); }
dt { color: var(--text-faint); font-size: var(--fine); }
dd { margin: 0; color: var(--text-quiet); overflow-wrap: anywhere; }
.claim { font-size: var(--fine); color: var(--text-faint); margin: 0; line-height: 1.5; }
</style>
