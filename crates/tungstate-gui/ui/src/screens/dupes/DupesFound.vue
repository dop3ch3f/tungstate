<!-- What the scan found, and what to do about it.
     Three promises live on this screen: a saving shown is a saving that is
     real, nothing moves on the strength of a sample, and every group keeps a
     copy whichever one you pick. -->
<script setup lang="ts">
import { computed } from "vue";
import { useDupes } from "../../state/useDupes";
import { bytes, when } from "../../lib/format";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import Empty from "../../ui/Empty.vue";

const d = useDupes();
const found = computed(() => d.found.value);

/** What the button will do, in the words it will say. */
const doing = computed(() =>
  d.action.value === "trash" && found.value?.can_trash ? "trash" : "set-aside",
);

async function go() {
  const answer = found.value;
  if (!answer) return;

  // Asked once, for the whole product, and remembered. The two are different
  // promises: one can be undone here and the other cannot.
  if (!d.action.value) {
    const chosen = await ask({
      title: "What should happen to the extra copies?",
      why: answer.can_trash
        ? "Set aside moves them inside the folder, and Put it back undoes it. The Trash is your desktop's: only the Finder can bring those back."
        : "This is not on this machine, so the desktop's Trash is not available. Extra copies are set aside inside the folder, and Put it back undoes it.",
      choices: answer.can_trash
        ? [
            { id: "set-aside", label: "Set them aside", look: "primary" },
            { id: "trash", label: "Send to the Trash", look: "danger" },
          ]
        : [{ id: "set-aside", label: "Set them aside", look: "primary" }],
    });
    if (!chosen.id) return;
    await d.chooseAction(chosen.id);
  }

  const action = doing.value === "trash" ? "Send to the Trash" : "Set aside";
  const detail = [
    `${d.chosenFiles.value} file(s), ${bytes(d.chosenBytes.value)}.`,
    doing.value === "trash"
      ? "Only the Finder can bring these back."
      : "They stay inside the folder, and Put it back undoes this.",
  ];
  if (d.anyUnsure.value) {
    detail.push("Every one is compared byte for byte first; any that differ are left alone.");
  }
  const confirmed = await ask({
    title: `${action} ${d.chosenFiles.value} file(s)?`,
    why: "One copy of each stays exactly where it is.",
    detail,
    choices: [
      { id: "no", label: "Not now" },
      { id: "yes", label: action, look: doing.value === "trash" ? "danger" : "primary" },
    ],
  });
  if (confirmed.id === "yes") await d.clear(doing.value);
}
</script>

<template>
  <div class="dz-found" v-if="found">
    <p class="dz-sum">
      <b class="num">{{ bytes(found.reclaimable) }}</b>
      in {{ found.extra_files }} extra file(s), out of {{ found.files }} looked at.
    </p>
    <Notice tone="hold" v-if="found.networked">
      This is over a network, so files were matched on samples rather than read
      in full. Anything you clear is compared byte for byte first.
    </Notice>
    <Notice tone="bad" v-if="d.problem.value">{{ d.problem.value }}</Notice>

    <Empty
      v-if="!found.groups.length && !found.folders.length"
      art="nothing-found"
      line="Nothing here is duplicated. Every file is the only copy of it."
    />

    <template v-else>
      <div class="dz-rules">
        <span class="dz-lbl">In every ticked group, keep</span>
        <Button @click="d.keepBy('newest')">the newest file</Button>
        <Button @click="d.keepBy('oldest')">the oldest file</Button>
      </div>

      <section class="dz-part" v-if="found.folders.length">
        <h2>Folders copied whole</h2>
        <div class="dz-group" v-for="group in found.folders" :key="group.id">
          <label class="dz-head">
            <input
              type="checkbox"
              :checked="d.picked.value.has(group.id)"
              @change="d.pick(group.id, ($event.target as HTMLInputElement).checked)"
            />
            <span class="dz-name">{{ group.keep }}</span>
            <span class="dz-meta">
              {{ group.files }} file(s), {{ bytes(group.bytes) }} each
            </span>
            <span class="dz-unsure" v-if="!group.sure">checked before anything moves</span>
          </label>
          <p class="path dz-copy" v-for="extra in group.extras" :key="extra">
            also at {{ extra }}
          </p>
        </div>
      </section>

      <section class="dz-part" v-if="found.groups.length">
        <h2>The same file, more than once</h2>
        <div class="dz-group" v-for="group in found.groups" :key="group.id">
          <label class="dz-head">
            <input
              type="checkbox"
              :checked="d.picked.value.has(group.id)"
              @change="d.pick(group.id, ($event.target as HTMLInputElement).checked)"
            />
            <span class="dz-name">{{ bytes(group.size) }} each</span>
            <span class="dz-meta">{{ group.extras.length + 1 }} copies</span>
            <span class="dz-unsure" v-if="!group.sure">checked before anything moves</span>
          </label>
          <!-- Every copy, and clicking one keeps that one instead. The kept
               copy is never in the list of things to deal with. -->
          <button
            v-for="copy in [group.kept, ...group.extras]"
            :key="copy.path"
            class="dz-row"
            :class="{ 'dz-keeps': d.kept(group) === copy.path }"
            @click="d.keep(group.id, copy.path)"
          >
            <span class="dz-tick">{{ d.kept(group) === copy.path ? "keep" : "" }}</span>
            <span class="path dz-path">{{ copy.path }}</span>
            <span class="dz-when">{{ copy.mtime ? when(Date.parse(copy.mtime)) : "" }}</span>
          </button>
        </div>
      </section>

      <section class="dz-part" v-if="found.linked.length">
        <h2>Two names for one file</h2>
        <p class="dz-note">These are hard links. Dealing with one frees nothing, so they are not offered.</p>
        <p class="path dz-copy" v-for="linked in found.linked" :key="linked.id">
          {{ linked.names.join("  =  ") }} ({{ bytes(linked.size) }})
        </p>
      </section>
    </template>

    <footer class="dz-foot" v-if="found.groups.length || found.folders.length">
      <Button look="primary" :disabled="!d.chosenFiles.value" @click="go()">
        {{ doing === "trash" ? "Send" : "Set aside" }} {{ d.chosenFiles.value }} file(s)
      </Button>
      <span class="dz-safe">
        {{ bytes(d.chosenBytes.value) }} comes back. One copy of each stays where it is.
      </span>
    </footer>
  </div>
</template>

<style scoped>
.dz-found { display: flex; flex-direction: column; gap: var(--s3); height: 100%; }
/* Room for the action bar, which floats over the end of the list. */
.dz-part:last-of-type { padding-bottom: 64px; }
.dz-sum { font-size: var(--body); color: var(--text-quiet); margin: 0; }
.dz-sum b { font-size: var(--title); color: var(--text); }
.dz-rules { display: flex; align-items: center; gap: var(--s2); }
.dz-lbl { font-size: var(--small); color: var(--text-faint); }
.dz-part { margin-top: var(--s4); }
.dz-part h2 { font-size: var(--small); font-weight: 700; margin: 0 0 var(--s2); text-transform: uppercase; letter-spacing: 0.04em; }
.dz-note { font-size: var(--fine); color: var(--text-faint); margin: 0 0 var(--s2); }
.dz-group { border-top: var(--bw) solid var(--edge); padding: var(--s2) 0; }
.dz-head { display: flex; align-items: center; gap: var(--s2); font-size: var(--small); cursor: pointer; }
.dz-name { font-weight: 600; }
.dz-meta { color: var(--text-faint); }
.dz-unsure { margin-left: auto; font-size: var(--fine); color: var(--hold); }
.dz-copy { color: var(--text-faint); margin: 2px 0 0 22px; }
.dz-row {
  display: flex;
  align-items: baseline;
  gap: var(--s2);
  width: 100%;
  margin: 2px 0 0;
  padding: 3px var(--s2) 3px 22px;
  font: inherit;
  text-align: left;
  color: var(--text-quiet);
  background: none;
  border: none;
  border-radius: var(--radius);
  cursor: pointer;
}
.dz-row:hover { background: var(--surface-hover); }
.dz-keeps { color: var(--text); background: var(--surface-raised); }
.dz-tick { width: 34px; flex: none; font-size: var(--fine); color: var(--accent); }
.dz-path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.dz-when { margin-left: auto; flex: none; font-size: var(--fine); color: var(--text-faint); }
.dz-foot {
  margin-top: auto;
  display: flex;
  align-items: center;
  gap: var(--s3);
  padding: var(--s3) var(--s4);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius-lg);
  background: var(--panel);
  position: sticky;
  bottom: 0;
}
.dz-safe { font-size: var(--small); color: var(--text-quiet); }
</style>
