<!-- What the scan found, in three panes: what kind of thing, which groups,
     and the file itself.

     Four promises live on this screen. A saving shown is a saving that is
     real. Nothing moves on the strength of a sample. Every group keeps a copy,
     whichever one you tick. And a resemblance is never ticked for you. -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useDupes } from "../../state/useDupes";
import { bytes } from "../../lib/format";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import DupesDrawers from "./DupesDrawers.vue";
import DupesList from "./DupesList.vue";
import DupesPreview from "./DupesPreview.vue";

const d = useDupes();
const found = computed(() => d.found.value);

/** Whether all three panes fit. Below this the preview folds into a toggle
 *  rather than squeezing three columns into the window's own minimum. */
const roomy = ref(true);
const showPreview = ref(false);
let watcher: MediaQueryList | null = null;
const measure = () => {
  roomy.value = watcher?.matches ?? true;
};

onMounted(() => {
  watcher = window.matchMedia("(min-width: 1040px)");
  watcher.addEventListener("change", measure);
  measure();
});
onUnmounted(() => watcher?.removeEventListener("change", measure));

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
  // Said out loud, every time, because a resemblance is arithmetic on pixels
  // rather than proof, and a guess plus an irreversible delete is the one
  // combination that can lose somebody a photograph.
  if (d.anyGuessed.value) {
    detail.push(
      doing.value === "trash"
        ? "Some of these were matched because they look alike, not because they are the same file. That is a resemblance, and the Trash cannot be undone from here."
        : "Some of these were matched because they look alike, not because they are the same file.",
    );
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
    <Notice tone="hold" v-if="found.networked">
      This is over a network, so files were matched on samples rather than read
      in full. Anything you clear is compared byte for byte first.
    </Notice>
    <Notice tone="bad" v-if="d.problem.value">{{ d.problem.value }}</Notice>

    <div class="none" v-if="!found.rows.length">
      <p>
        Nothing here is a copy of anything else here.
        {{ found.files }} file(s) looked at.
      </p>
      <Button @click="d.again()">Look somewhere else</Button>
    </div>

    <template v-else>
      <div class="rules">
        <span class="rules-lbl">Keep</span>
        <Button look="link" @click="d.keepBy('newest')">the newest</Button>
        <Button look="link" @click="d.keepBy('oldest')">the oldest</Button>
        <Button look="link" @click="d.keepBy('biggest')">the biggest</Button>
        <span class="rules-gap"></span>
        <Button look="link" @click="d.keepBy('all')">Tick every extra</Button>
        <Button look="link" @click="d.keepBy('none')">Tick nothing</Button>
        <Button
          v-if="!roomy"
          look="link"
          @click="showPreview = !showPreview"
        >{{ showPreview ? "Hide the file" : "Show the file" }}</Button>
      </div>

      <div class="panes" :class="{ wide: roomy, peek: showPreview && !roomy }">
        <DupesDrawers class="pane-left" />
        <DupesList class="pane-mid" />
        <DupesPreview v-if="roomy || showPreview" class="pane-right" />
      </div>

      <footer class="foot">
        <p class="foot-sum">
          <b>{{ bytes(found.reclaimable) }}</b> in
          {{ found.extra_files }} extra file(s), out of {{ found.files }} looked at.
          <span v-if="found.unchecked.length" class="foot-note">
            {{ found.unchecked.length }} could not be looked at.
          </span>
        </p>
        <Button
          look="primary"
          :disabled="!d.chosenFiles.value"
          @click="go()"
        >
          {{ doing === "trash" ? "Send" : "Set aside" }}
          {{ d.chosenFiles.value }} file(s)
        </Button>
      </footer>
    </template>
  </div>
</template>

<style scoped>
.dz-found { display: flex; flex-direction: column; gap: var(--s3); flex: 1; min-height: 0; }
.none { display: flex; flex-direction: column; align-items: flex-start; gap: var(--s3); }
.none p { font-size: var(--body); color: var(--text-quiet); margin: 0; }
.rules { display: flex; align-items: center; gap: var(--s3); flex-wrap: wrap; }
.rules-lbl { font-size: var(--fine); color: var(--text-faint); text-transform: uppercase; letter-spacing: 0.05em; }
.rules-gap { flex: 1; }
.panes {
  display: grid;
  grid-template-columns: 170px minmax(0, 1fr);
  gap: var(--s3);
  flex: 1;
  min-height: 0;
  border-top: var(--bw) solid var(--edge);
  padding-top: var(--s3);
}
.wide { grid-template-columns: 190px minmax(280px, 1fr) minmax(260px, 0.85fr); }
.peek { grid-template-columns: 170px minmax(0, 1fr) minmax(220px, 0.8fr); }
.pane-left { min-height: 0; }
.pane-mid { min-height: 0; }
.pane-right { min-height: 0; }
.foot {
  display: flex;
  align-items: center;
  gap: var(--s4);
  padding-top: var(--s3);
  border-top: var(--bw) solid var(--edge);
}
.foot-sum { flex: 1; font-size: var(--small); color: var(--text-quiet); margin: 0; }
.foot-note { color: var(--text-faint); }
</style>
