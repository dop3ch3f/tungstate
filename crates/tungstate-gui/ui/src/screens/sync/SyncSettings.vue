<!-- How a sync behaves, as choices rather than flags: whether deletions travel,
     what a removed copy becomes, what happens when two folders disagree, and
     whether it runs when the app opens. Which way things go and between which
     folders are the new-sync page's, because changing either later would make
     what the sync remembers describe a different sync. -->
<script setup lang="ts">
import { EXACTLY, REMOVING, CONFLICTS, LAUNCH, FIRST_CHECK } from "../../lib/syncwords";
import { CHOICES, type SyncSettingsForm } from "../../engine/types";

const form = defineModel<SyncSettingsForm>({ required: true });
const set = (patch: Partial<SyncSettingsForm>) => (form.value = { ...form.value, ...patch });

function exact(on: boolean) {
  // Deleting outright only exists with deletions travelling; turning them
  // off takes the delete back with it rather than leaving a refusal to read.
  set(on ? { exact: true } : { exact: false, on_remove: "set-aside" });
}
</script>

<template>
  <div class="sy-set">
    <fieldset class="sy-group">
      <legend>When a file is deleted</legend>
      <label v-for="c in EXACTLY" :key="String(c.id)" class="sy-card" :class="{ 'sy-on': form.exact === c.id }">
        <input type="radio" :checked="form.exact === c.id" @change="exact(c.id)" />
        <b>{{ c.label }}</b>
        <span>{{ c.line }}</span>
      </label>
    </fieldset>

    <fieldset class="sy-group" v-if="form.exact">
      <legend>What a removed copy becomes</legend>
      <label v-for="c in REMOVING" :key="c.id" class="sy-card" :class="{ 'sy-on': form.on_remove === c.id, 'sy-warn': c.id === 'delete' }">
        <input type="radio" :checked="form.on_remove === c.id" @change="set({ on_remove: c.id })" />
        <b>{{ c.label }}</b>
        <span>{{ c.line }}</span>
      </label>
    </fieldset>

    <fieldset class="sy-group">
      <legend>When two folders changed the same file differently</legend>
      <label v-for="c in CONFLICTS" :key="c.id" class="sy-card" :class="{ 'sy-on': form.on_conflict === c.id }">
        <input type="radio" :checked="form.on_conflict === c.id" @change="set({ on_conflict: c.id })" />
        <b>{{ c.label }}</b>
        <span>{{ c.line }}</span>
      </label>
    </fieldset>

    <fieldset class="sy-group">
      <legend>When it runs</legend>
      <label v-for="c in LAUNCH" :key="c.id" class="sy-card" :class="{ 'sy-on': form.launch === c.id }">
        <input type="radio" :checked="form.launch === c.id" @change="set({ launch: c.id })" />
        <b>{{ c.label }}</b>
        <span v-if="c.line">{{ c.line }}</span>
      </label>
    </fieldset>

    <details class="sy-more">
      <summary>More</summary>
      <div class="sy-more-body">
        <label class="sy-field">
          <span>Two copies meeting for the first time</span>
          <select :value="form.first_check" @change="set({ first_check: ($event.target as HTMLSelectElement).value })">
            <option v-for="c in FIRST_CHECK" :key="c.id" :value="c.id">{{ c.label }}: {{ c.line }}</option>
          </select>
        </label>
        <label class="sy-field">
          <span>How each copy is checked</span>
          <select :value="form.verify" @change="set({ verify: ($event.target as HTMLSelectElement).value })">
            <option v-for="[id, words] in CHOICES.verify" :key="id" :value="id">{{ words }}</option>
          </select>
        </label>
        <label class="sy-field">
          <span>Wait this many seconds after a file last changed</span>
          <input
            type="number"
            min="0"
            :value="form.cooldown_secs"
            @change="set({ cooldown_secs: Math.max(0, Number(($event.target as HTMLInputElement).value) || 0) })"
          />
        </label>
      </div>
    </details>
  </div>
</template>

<style scoped>
.sy-set { display: flex; flex-direction: column; gap: var(--s4); }
.sy-group { border: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: var(--s2); }
.sy-group legend { font-size: var(--small); font-weight: 700; margin-bottom: var(--s2); padding: 0; }
.sy-card {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: var(--s3) var(--s3) var(--s3) 34px;
  background: var(--panel);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius);
  cursor: pointer;
  font-size: var(--small);
}
.sy-card input { position: absolute; left: var(--s3); top: 14px; margin: 0; accent-color: var(--control); }
.sy-card span { font-size: var(--fine); color: var(--text-faint); line-height: 1.45; }
.sy-on { box-shadow: inset 0 0 0 var(--bw) var(--control), var(--lift); }
.sy-warn b { color: var(--bad); }
.sy-more summary { font-size: var(--small); color: var(--text-quiet); cursor: pointer; }
.sy-more-body { display: grid; gap: var(--s3); margin-top: var(--s3); max-width: 60ch; }
.sy-field { display: flex; flex-direction: column; gap: 5px; font-size: var(--small); }
.sy-field span { font-weight: 600; }
:global([data-theme="retro"] .sy-card.sy-on) { box-shadow: inset 0 0 0 var(--bw) var(--edge), var(--lift); background: var(--panel); }
</style>
