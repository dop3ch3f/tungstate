<!-- A sync while it runs, pair by pair, and what it did when it stops. The
     way back is on this screen: a finished run can be put back from here. -->
<script setup lang="ts">
import { computed } from "vue";
import { useSync } from "../../state/useSync";
import { files, syncCopied, syncMissed, syncRenamed, syncTookOff } from "../../lib/counts";
import { bytes } from "../../lib/format";
import { toneOfOutcome } from "../../lib/tone";
import { missed } from "../../lib/syncwords";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import StopPair from "../../ui/StopPair.vue";
import Working from "../../ui/Working.vue";
import SyncUndoneNotice from "./SyncUndoneNotice.vue";

const s = useSync();
const ran = computed(() => s.ran.value);

/** Plain words for an engine outcome; an unknown one is shown as itself. */
const WORD: Record<string, string> = {
  waiting: "waiting",
  live: "copying",
  checking: "checking it arrived",
  transferred: "verified",
  "already-present": "already there",
  quarantined: "set aside",
  skipped: "left alone",
  failed: "failed",
};
const word = (state: string) => WORD[state] ?? state;
// A lookup, because the CSS checker cannot follow a computed class name.
const DOT = { plain: "rn-plain", live: "rn-live", ok: "rn-ok", hold: "rn-hold", bad: "rn-bad" } as const;
const dot = (state: string) =>
  state === "live" || state === "checking" ? DOT.live : DOT[toneOfOutcome(state)];
/** What the run did, as one paragraph. Built here rather than in the
 *  template, where the spaces between optional sentences get dropped. */
const said = computed(() => {
  const r = ran.value;
  if (!r) return "";
  const parts = [r.stopped ? "Stopped when you asked." : "Finished.", `Copied ${files(syncCopied(r))}.`];
  if (r.taken_off) parts.push(`${files(syncTookOff(r))} ${r.reversible ? "set aside" : "set aside or deleted"}.`);
  if (r.renamed) parts.push(`${files(syncRenamed(r))} renamed.`);
  if (r.missed.length) parts.push(`${files(syncMissed(r))} not done; it will be tried next time.`);
  return parts.join(" ");
});
const copiedBytes = computed(() =>
  s.rows.value.filter((r) => r.state === "transferred").reduce((n, r) => n + r.size, 0),
);

async function putBack() {
  const plan = ran.value?.plan;
  if (plan == null || !s.current.value) return;
  const answer = await ask({
    title: "Put this run back?",
    why: "Copies it made are set aside again, and anything it set aside goes back where it was, on every folder.",
    choices: [
      { id: "cancel", label: "Cancel" },
      { id: "go", label: "Put back", look: "primary" },
    ],
  });
  if (answer.id === "go") await s.putBack(s.current.value.name, plan);
}
</script>

<template>
  <div class="rn">
    <h2 class="rn-title">{{ s.current.value?.name }}</h2>

    <div class="rn-readout">
      <span class="rn-mass num">{{ bytes(copiedBytes) }}</span>
      <span class="rn-of">copied</span>
      <span class="rn-of" v-if="s.rows.value.length">
        <b class="num">{{ s.settled.value }}</b> of <b class="num">{{ s.rows.value.length }}</b> files
      </span>
      <span class="rn-leg" v-if="s.leg.value && !ran">{{ s.leg.value.from }} → {{ s.leg.value.to }}</span>
      <span class="rn-gap"></span>
      <StopPair v-if="!ran" :stopping="s.stopping.value" :halting="s.halting.value" @stop="s.stop()" @now="s.stopNow()" />
    </div>

    <Working v-if="!ran && !s.rows.value.length" what="Setting things aside and renaming first" note="then each pair of folders is copied in turn" />

    <div class="rn-ledger" v-if="s.rows.value.length">
      <div class="rn-line" v-for="row in s.rows.value" :key="`${row.leg}:${row.path}`">
        <span class="rn-dot" :class="dot(row.state)"></span>
        <span class="path rn-file">{{ row.path }}</span>
        <span class="rn-word">{{ word(row.state) }}</span>
        <span class="num rn-size">{{ bytes(row.size) }}</span>
        <span class="rn-bar" v-if="row.state === 'live' || row.state === 'checking'">
          <i :style="{ width: (row.size ? (row.done / row.size) * 100 : 0) + '%' }"></i>
        </span>
      </div>
    </div>

    <template v-if="ran">
      <Notice :tone="ran.missed.length || ran.legs.some((l) => l.failed) ? 'hold' : 'plain'">
        {{ said }}
      </Notice>
      <ul class="rn-missed" v-if="ran.missed.length">
        <li v-for="m in ran.missed" :key="`${m.member}${m.path}`">
          <span class="path">{{ m.path }}</span>
          <span class="rn-why">{{ missed(m) }}</span>
        </li>
      </ul>
      <SyncUndoneNotice />
      <Notice tone="bad" v-if="s.problem.value">{{ s.problem.value }}</Notice>
      <footer class="rn-foot">
        <Button look="primary" v-if="ran.plan != null && ran.reversible && !s.undone.value" @click="putBack()">
          {{ s.busy.value === ran.plan ? "Putting back…" : "Put it back" }}
        </Button>
        <span class="rn-hint" v-else-if="ran.plan != null && !ran.reversible">This run deleted files outright, so it cannot be put back.</span>
        <Button look="link" @click="s.back()">Done</Button>
      </footer>
    </template>
  </div>
</template>

<style scoped>
.rn { display: flex; flex-direction: column; gap: var(--s3); min-height: 0; }
.rn-title { font-size: var(--title); font-weight: 700; margin: 0; }
.rn-readout { display: flex; align-items: baseline; gap: var(--s5); flex-wrap: wrap; }
.rn-mass { font-size: 30px; font-weight: 500; letter-spacing: -0.02em; }
.rn-of { font-size: var(--small); color: var(--text-quiet); }
.rn-of b { color: var(--text); font-weight: 600; }
.rn-leg { font-size: var(--small); color: var(--text-faint); }
.rn-gap { flex: 1; }
.rn-ledger { max-height: 50vh; overflow-y: auto; }
.rn-line {
  display: grid;
  grid-template-columns: 9px minmax(0, 1fr) 140px 88px;
  gap: var(--s2);
  align-items: center;
  padding: 5px 0;
  font-size: var(--fine);
  border-bottom: var(--bw) solid var(--edge);
}
.rn-dot { width: 7px; height: 7px; border-radius: 50%; }
.rn-plain { background: var(--text-faint); }
.rn-live { background: var(--accent); }
.rn-ok { background: var(--ok); }
.rn-hold { background: var(--hold); }
.rn-bad { background: var(--bad); }
.rn-file { color: var(--text-quiet); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.rn-word { color: var(--text-faint); }
.rn-size { text-align: right; color: var(--text-faint); }
.rn-bar { grid-column: 1 / -1; height: 2px; background: var(--edge); border-radius: 1px; overflow: hidden; }
.rn-bar i { display: block; height: 100%; background: var(--accent); }
.rn-missed { list-style: none; margin: 0; padding: 0; font-size: var(--fine); }
.rn-missed li { display: flex; gap: var(--s3); padding: 3px 0; color: var(--text-quiet); }
.rn-why { color: var(--hold); margin-left: auto; }
.rn-foot { display: flex; align-items: center; gap: var(--s3); }
.rn-hint { font-size: var(--fine); color: var(--text-faint); }
</style>
