<!-- A copy or move that stopped before it finished: the phone closed
     Tungstate, the battery died, or it was paused. Resume picks it up from
     the journal: files already sent and checked are not sent again, and a
     half-sent one starts over. Clean up removes what a half-sent file left at
     the far side and leaves everything not sent where it is. -->
<script setup lang="ts">
import type { InterruptedRun } from "../../engine/types";
import { useTransfer } from "../../state/useTransfer";
import { placeName } from "../lib/paths";
import { useClipboard } from "../state/useClipboard";
import { useStorage } from "../state/useStorage";
import PButton from "../ui/PButton.vue";
import PTile from "../ui/PTile.vue";

const t = useTransfer();
const board = useClipboard();
const store = useStorage();

/** "Area51 › Photos", never the engine's `Area51:Photos`. */
const where = (r: InterruptedRun) => placeName(r.destination, store.tops.value);

const verb = (r: InterruptedRun) => (r.removes_originals ? "move" : "copy");

async function resume(r: InterruptedRun) {
  const got = await t.resume(r.link);
  if (got) {
    board.follow({ job: got.job, op: verb(r), what: "what was left", to: where(r), destination: "", paths: [], resumed: true });
  }
}
</script>

<template>
  <div v-for="r in t.stranded.value" :key="r.link" class="resume">
    <div class="top">
      <PTile art="drain" tint="var(--tint-drain)" light :size="36" />
      <div class="words">
        <b>A {{ verb(r) }} to {{ where(r) }} stopped before it finished</b>
        <!-- No count of what is left: the journal knows only the files that
             were mid-send, and "3 files left" of 200 would be a lie. -->
        <span class="quiet">Resume sends what is left. Anything already sent and checked is not sent again.</span>
      </div>
    </div>
    <div class="pair">
      <PButton :busy="t.settling(r.link)" @press="resume(r)">Resume</PButton>
      <PButton quiet :disabled="t.settling(r.link)" @press="t.discard(r.link)">Clean up</PButton>
    </div>
  </div>
</template>

<style scoped>
.resume { display: flex; flex-direction: column; gap: 12px; margin: 0 0 14px; padding: 12px; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; box-shadow: 4px 4px 0 var(--edge); }
.top { display: flex; gap: 12px; align-items: flex-start; }
.words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
b { font-size: 16px; line-height: 1.3; }
.quiet { font-size: 14px; line-height: 1.4; color: var(--text-quiet); }
.pair { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
</style>
