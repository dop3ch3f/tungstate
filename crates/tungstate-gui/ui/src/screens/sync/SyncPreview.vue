<!-- What a run would do, before it does anything: per folder, what arrives,
     what leaves, what is removed and what is renamed, each its own number.
     Conflicts are asked about here, and the answers are previewed again at
     once, so the numbers are always the run that Run would start. -->
<script setup lang="ts">
import { computed, ref } from "vue";
import { useSync } from "../../state/useSync";
import {
  arriving, files, holds, inLeg, leaving, listedRemovals, parking, readToDecide, removing,
  renaming, replacing,
} from "../../lib/counts";
import { bytes, when } from "../../lib/format";
import { CONFLICTS, refusal, removedBecause, why } from "../../lib/syncwords";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import Pixels from "../../ui/Pixels.vue";

const s = useSync();
const p = computed(() => s.preview.value!);
const sync = computed(() => s.current.value!);
const SHOWN = 10;
const allRemovals = ref(false);

/** Folders whose version could be kept for a conflict: the members that
 *  changed it, as the engine named them. */
const keepers = computed(() => {
  const names = new Set<string>();
  for (const c of s.seen.value) for (const v of c.versions) names.add(v.member);
  return [...names];
});
const choiceOf = (path: string) => s.choices.value[path] ?? "leave";
const unanswered = computed(() => CONFLICTS.find((c) => c.id === sync.value.on_conflict)?.label ?? "");
const otherLeft = computed(() => p.value.left_alone.filter((l) => l.why !== "conflict"));
const deleting = computed(() => p.value.members.reduce((n, m) => n + m.deleting, 0));
const blocked = computed(() => p.value.refusals.length > 0 && !s.confirmed.value);
/** Removals by folder and reason, so the reason is said once per group
 *  rather than on every row. */
const groups = computed(() => {
  const by = new Map<string, { key: string; why: string; rows: typeof p.value.removed }>();
  for (const r of p.value.removed) {
    const key = `${r.member}|${r.because}|${r.gone}`;
    if (!by.has(key)) by.set(key, { key, why: removedBecause(r, p.value.members.map((m) => m.name)), rows: [] });
    by.get(key)!.rows.push(r);
  }
  return [...by.values()];
});
const title = computed(() =>
  s.forgetting.value.length
    ? `Forget ${s.forgetting.value.join(", ")}`
    : `What a run of ${sync.value.name} would do`,
);
</script>

<template>
  <div class="sp">
    <h2 class="sp-title">{{ title }}</h2>

    <div class="sp-empty" v-if="p.empty">
      <Pixels of="in-step" :size="64" class="sp-art" />
      <p>Nothing to do: every folder already holds what it should.</p>
    </div>

    <template v-else>
      <ul class="sp-members">
        <li v-for="m in p.members" :key="m.name" class="sp-member">
          <b>{{ m.name }}</b>
          <span class="path sp-at">{{ m.at }}</span>
          <span class="sp-line" v-if="m.arriving">{{ files(arriving(m)) }} arriving, {{ bytes(m.arriving_bytes) }}</span>
          <span class="sp-line" v-if="m.leaving">{{ files(leaving(m)) }} copied to the others</span>
          <span class="sp-line" v-if="m.replacing">
            {{ files(replacing(m)) }} replaced, the old {{ m.replacing === 1 ? "version" : "versions" }} {{ m.deleting ? "deleted" : "set aside" }}
          </span>
          <span class="sp-line sp-off" v-if="m.removing">{{ files(removing(m)) }} removed, {{ m.deleting ? "deleted for good" : "kept in the set-aside area" }}</span>
          <span class="sp-line" v-if="m.renaming">{{ files(renaming(m)) }} renamed</span>
          <span class="sp-line" v-if="m.parking">{{ files(parking(m)) }} parked beside the {{ m.parking === 1 ? "file it conflicts with" : "files they conflict with" }}</span>
          <span class="sp-line sp-quiet" v-if="!m.arriving && !m.leaving && !m.replacing && !m.removing && !m.renaming && !m.parking">nothing changes here</span>
          <span class="sp-holds">holds {{ files(holds(m)) }}</span>
        </li>
      </ul>

      <!-- Said only when it costs something: bytes between two places that
           are both elsewhere come through this Mac. -->
      <ul class="sp-legs" v-if="p.legs.some((l) => l.through_here)">
        <li v-for="leg in p.legs.filter((l) => l.through_here)" :key="`${leg.from}>${leg.to}`">
          {{ leg.from }} → {{ leg.to }}: {{ files(inLeg(leg)) }}, {{ bytes(leg.bytes) }}<template v-if="leg.through_here">, through this Mac</template>
        </li>
      </ul>
    </template>

    <!-- What needs a yes comes before any list, so it is read first. -->
    <Notice tone="bad" v-if="!p.reversible">
      This run cannot be put back: it deletes {{ deleting }} {{ deleting === 1 ? "file" : "files" }} outright.
    </Notice>
    <template v-if="p.refusals.length">
      <Notice tone="hold" v-for="r in p.refusals" :key="`${r.kind}${r.member}`">{{ refusal(r) }}</Notice>
      <label class="sp-yes">
        <input type="checkbox" v-model="s.confirmed.value" />
        I have looked at this, and it is what I mean. Run it anyway.
      </label>
    </template>
    <section class="sp-panel" v-if="s.seen.value.length">
      <h2><Pixels of="conflict" :size="20" class="sp-icon" /> Changed differently on more than one folder</h2>
      <p class="sp-note">Choose for each, or for all of them. Any left alone: {{ unanswered.toLowerCase() }}.</p>
      <div class="sp-all">
        <span>For all of them:</span>
        <Button v-for="k in keepers" :key="k" @click="s.chooseAll(k)">Keep {{ k }}'s</Button>
        <Button @click="s.chooseAll('both')">{{ keepers.length > 2 ? "Keep all" : "Keep both" }}</Button>
        <Button look="link" @click="s.chooseAll('leave')">Leave them</Button>
      </div>
      <ul class="sp-conflicts">
        <li v-for="c in s.seen.value" :key="c.path" class="sp-conflict">
          <span class="path sp-cpath">{{ c.path }}</span>
          <span class="sp-versions">
            <span v-for="v in c.versions" :key="v.member">
              {{ v.member }}: {{ bytes(v.size) }}<template v-if="v.modified">, {{ when(v.modified) }}</template>
            </span>
          </span>
          <span class="sp-pick">
            <button
              v-for="v in c.versions"
              :key="v.member"
              class="sp-seg"
              :class="{ 'sp-seg-on': choiceOf(c.path) === v.member }"
              @click="s.choose(c.path, v.member)"
            >Keep {{ v.member }}'s</button>
            <button class="sp-seg" :class="{ 'sp-seg-on': choiceOf(c.path) === 'both' }" @click="s.choose(c.path, 'both')">{{ c.versions.length > 2 ? "Keep all" : "Keep both" }}</button>
            <button class="sp-seg" :class="{ 'sp-seg-on': choiceOf(c.path) === 'leave' }" @click="s.choose(c.path, 'leave')">Leave it</button>
          </span>
        </li>
      </ul>
    </section>

    <section class="sp-panel" v-if="p.removed.length">
      <h2>Removed: {{ files(listedRemovals(p)) }}</h2>
      <div v-for="g in groups" :key="g.key" class="sp-group">
        <p class="sp-why">{{ g.why }}</p>
        <ul class="sp-list">
          <li v-for="r in (allRemovals ? g.rows : g.rows.slice(0, SHOWN))" :key="r.path">
            <span class="path">{{ r.path }}</span>
          </li>
        </ul>
        <Button look="link" v-if="g.rows.length > SHOWN && !allRemovals" @click="allRemovals = true">
          and {{ g.rows.length - SHOWN }} more
        </Button>
      </div>
    </section>

    <section class="sp-panel" v-if="otherLeft.length">
      <h2>Left alone</h2>
      <ul class="sp-list">
        <li v-for="l in otherLeft" :key="`${l.path}${l.why}`">
          <span class="path">{{ l.path }}</span>
          <span class="sp-from">{{ why(l) }}</span>
        </li>
      </ul>
    </section>

    <!-- Said only when it is the reason a run is slow: a folder with no
         times, or samples. Reads to recognise a move are quick and routine. -->
    <p class="sp-read" v-if="p.read.no_times || p.read.sampled">
      Read {{ files(readToDecide(p)) }} to decide<template v-if="p.read.sampled">, {{ p.read.sampled }} by samples</template><template v-if="p.read.no_times">, {{ p.read.no_times }} because a folder keeps no times</template><template v-if="p.read.moves">, {{ p.read.moves }} to tell a moved file from a new one</template>.
    </p>

    <Notice tone="bad" v-if="s.problem.value">{{ s.problem.value }}</Notice>

    <footer class="sp-foot">
      <Button look="primary" v-if="!p.empty" :disabled="blocked || s.rechecking.value" @click="s.run()">
        {{ s.rechecking.value ? "Checking again…" : s.forgetting.value.length ? "Forget it" : "Run" }}
      </Button>
      <Button look="link" @click="s.back()">{{ p.empty ? "Back" : "Not now" }}</Button>
      <span class="sp-hint" v-if="!p.empty && p.reversible">Every run can be put back afterwards, from this sync's page.</span>
    </footer>
  </div>
</template>

<style scoped>
.sp { display: flex; flex-direction: column; gap: var(--s4); }
.sp-title { font-size: var(--title); font-weight: 700; margin: 0; }
h2 { display: flex; align-items: center; gap: var(--s2); font-size: var(--small); font-weight: 700; margin: 0 0 var(--s2); }
.sp-empty { display: flex; align-items: center; gap: var(--s4); font-size: var(--body); color: var(--text-quiet); }
.sp-art { color: var(--control); }
.sp-icon { color: var(--control); }
.sp-members { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(210px, 1fr)); gap: var(--s3); }
.sp-member {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: var(--s3);
  background: var(--panel);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius-lg);
  box-shadow: var(--lift);
  font-size: var(--small);
  min-width: 0;
}
.sp-at { font-size: var(--fine); color: var(--text-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; margin-bottom: var(--s1); }
.sp-line { font-size: var(--small); }
.sp-off { color: var(--hold); font-weight: 600; }
.sp-quiet { color: var(--text-faint); }
.sp-holds { font-size: var(--fine); color: var(--text-faint); margin-top: var(--s1); }
.sp-legs { margin: 0; padding-left: 1.1em; font-size: var(--small); color: var(--text-quiet); line-height: 1.6; }
.sp-panel {
  padding: var(--s3) var(--s4);
  background: var(--panel);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius-lg);
  box-shadow: var(--lift-panel);
}
.sp-note { font-size: var(--fine); color: var(--text-faint); margin: 0 0 var(--s3); }
.sp-all { display: flex; align-items: center; gap: var(--s2); flex-wrap: wrap; font-size: var(--small); margin-bottom: var(--s3); }
.sp-conflicts, .sp-list { list-style: none; margin: 0; padding: 0; border-top: var(--bw) solid var(--rule); }
.sp-conflict { display: grid; grid-template-columns: minmax(0, 1fr) auto; grid-template-areas: "path pick" "versions pick"; gap: 2px var(--s3); padding: 7px 0; border-bottom: var(--bw) solid var(--rule); align-items: center; }
.sp-cpath { grid-area: path; font-size: var(--small); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sp-versions { grid-area: versions; display: flex; gap: var(--s3); font-size: var(--fine); color: var(--text-faint); flex-wrap: wrap; }
.sp-pick { grid-area: pick; display: flex; }
.sp-seg {
  font: inherit;
  font-size: var(--fine);
  padding: 4px var(--s2);
  background: var(--panel);
  color: var(--text-quiet);
  border: var(--bw) solid var(--edge);
  margin-left: calc(-1 * var(--bw));
  cursor: pointer;
  white-space: nowrap;
}
.sp-seg:first-child { border-radius: var(--radius) 0 0 var(--radius); margin-left: 0; }
.sp-seg:last-child { border-radius: 0 var(--radius) var(--radius) 0; }
.sp-seg-on { background: var(--control); color: var(--control-ink); font-weight: 700; }
.sp-list li { display: flex; gap: var(--s3); justify-content: space-between; padding: 5px 0; border-bottom: var(--bw) solid var(--rule); font-size: var(--small); }
.sp-from { color: var(--text-faint); font-size: var(--fine); white-space: nowrap; }
.sp-group + .sp-group { margin-top: var(--s3); }
.sp-why { font-size: var(--small); color: var(--text-quiet); margin: 0 0 var(--s2); }
.sp-read { font-size: var(--fine); color: var(--text-faint); margin: 0; }
.sp-yes { display: flex; align-items: center; gap: var(--s2); font-size: var(--small); }
.sp-foot { display: flex; align-items: center; gap: var(--s3); flex-wrap: wrap; }
.sp-hint { font-size: var(--fine); color: var(--text-faint); }
:global([data-theme="retro"] .sp-seg-on) { background: var(--chosen); color: var(--chosen-ink); box-shadow: inset 0 0 0 var(--bw) var(--edge), var(--lift); }
</style>
