<!-- What the scan found, in three panes: what kind of thing, which groups,
     and the file itself.

     Four promises live on this screen. A saving shown is a saving that is
     real. Nothing moves on the strength of a sample. Every group keeps a copy,
     whichever one you tick. And a resemblance is never ticked for you. -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useDupes } from "../../state/useDupes";
import { bytes } from "../../lib/format";
import { extraIn, files, lookedAt } from "../../lib/counts";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import ActionBar from "../../ui/ActionBar.vue";
import Segmented from "../../ui/Segmented.vue";
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
      title: `Set aside ${files(d.chosenFiles.value)}, or send them to the Trash?`,
      why: answer.can_trash
        ? "Set aside keeps them inside the folder, where Put it back can bring them back. From the Trash, only the Finder can. This is asked once and remembered."
        : "This is not on this Mac, so there is no Trash for it. The extra copies are set aside inside the folder, where Put it back can bring them back.",
      choices: answer.can_trash
        ? [
            { id: "cancel", label: "Cancel" },
            { id: "trash", label: "Send to the Trash", look: "danger" },
            { id: "set-aside", label: "Set them aside", look: "primary" },
          ]
        : [
            { id: "cancel", label: "Cancel" },
            { id: "set-aside", label: "Set them aside", look: "primary" },
          ],
    });
    if (!chosen.id || chosen.id === "cancel") return;
    await d.chooseAction(chosen.id);
  }

  const action = doing.value === "trash" ? "Send to the Trash" : "Set aside";
  const detail = [
    `${files(d.chosenFiles.value)}, ${bytes(d.chosenBytes.value)}.`,
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
    title: `${action} ${files(d.chosenFiles.value)}?`,
    why: "One copy of each stays exactly where it is.",
    detail,
    choices: [
      { id: "no", label: "Not now" },
      { id: "yes", label: action, look: doing.value === "trash" ? "danger" : "primary" },
    ],
  });
  if (confirmed.id === "yes") await d.clear(doing.value);
}
// Suggested is the copy the scan chose and explains; the ticks start there.
const KEEP = [
  { id: "suggested", label: "Suggested" },
  { id: "newest", label: "The newest" },
  { id: "oldest", label: "The oldest" },
  { id: "biggest", label: "The biggest" },
] as const;
</script>

<template>
  <div class="dz-found" v-if="found">
    <Notice tone="hold" v-if="found.networked">
      This is over a network, so files were matched on samples rather than read
      in full. Anything you clear is compared byte for byte first.
    </Notice>
    <Notice v-if="found.online_only">
      {{ found.online_only }} {{ found.online_only === 1 ? "file is" : "files are" }} kept online only by
      the drive's app, so {{ found.online_only === 1 ? "it was" : "they were" }} left out: comparing
      {{ found.online_only === 1 ? "it" : "them" }} would download {{ found.online_only === 1 ? "it" : "them" }}.
      Make them available offline in the app to include them.
    </Notice>
    <Notice tone="bad" v-if="d.problem.value">{{ d.problem.value }}</Notice>

    <div class="none" v-if="!found.rows.length">
      <p>
        Nothing here is a copy of anything else here.
        {{ files(lookedAt(found)) }} looked at.
      </p>
      <Button @click="d.again()">Look somewhere else</Button>
    </div>

    <template v-else>
      <div class="rules">
        <span class="rules-lbl">Keep</span>
        <Segmented
          label="Which copy to keep"
          :options="KEEP"
          :model-value="d.keptBy.value ?? 'suggested'"
          @update:model-value="(k) => d.keepBy(k === 'suggested' ? 'all' : k)"
        />
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

      <ActionBar pinned>
        <template #say><p class="foot-sum">
          <b>{{ files(d.chosenFiles.value) }} ticked, {{ bytes(d.chosenBytes.value) }}.</b>{{ " " }}<span class="foot-note">{{ extraIn(found) }} extra {{ extraIn(found) === 1 ? "file" : "files" }} in {{ lookedAt(found) }} looked at.</span>{{ " " }}<span v-if="found.unchecked.length" class="foot-note">
            {{ found.unchecked.length }} could not be looked at.
          </span>
        </p></template>
        <Button
          look="primary"
          :disabled="!d.chosenFiles.value"
          @click="go()"
        >
          {{ doing === "trash" ? "Send" : "Set aside" }}
          {{ files(d.chosenFiles.value) }}
        </Button>
      </ActionBar>
    </template>
  </div>
</template>

<style scoped>
.dz-found { display: flex; flex-direction: column; gap: var(--s3); flex: 1; min-height: 0; }
.none { display: flex; flex-direction: column; align-items: flex-start; gap: var(--s3); }
.none p { font-size: var(--body); color: var(--text-quiet); margin: 0; }
.rules { display: flex; align-items: center; gap: var(--s3); flex-wrap: wrap; }
.rules-lbl { font-size: var(--fine); font-weight: 600; color: var(--text-quiet); }
.rules-gap { flex: 1; }
.panes {
  display: grid;
  grid-template-columns: 170px minmax(0, 1fr);
  gap: var(--s3);
  flex: 1;
  min-height: 0;
  border-top: 1px solid var(--rule);
  padding-top: var(--s3);
}
/* The list is where the choosing happens, so it gets the room. */
.wide { grid-template-columns: 180px minmax(320px, 1.35fr) minmax(240px, 0.8fr); }
.peek { grid-template-columns: 170px minmax(0, 1fr) minmax(220px, 0.8fr); }
.pane-left { min-height: 0; }
.pane-mid { min-height: 0; }
.pane-right { min-height: 0; }
.foot-sum { margin: 0; }
.foot-note { color: var(--text-faint); }
</style>
