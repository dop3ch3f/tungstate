<!-- What will happen to these files, and the six choices that decide it.
     Shown before anything moves: property 1 applies to the drain half too. -->
<script setup lang="ts">
import { computed, onMounted, ref, shallowRef } from "vue";
import { transfers } from "../../engine/commands";
import { CHOICES, type Leg, type Preview, type TransferRequest } from "../../engine/types";
import { bytes } from "../../lib/format";
import Sheet from "../../ui/Sheet.vue";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";

const props = defineProps<{ legs: Leg[]; intent: "move" | "copy" }>();
const emit = defineEmits<{ dismiss: []; go: [request: TransferRequest] }>();

const policy = ref(props.intent === "move" ? "delete" : "keep");
const verify = ref("hash");
const onConflict = ref("quarantine");
const order = ref("largest-first");
const saveAs = ref("");
const preview = shallowRef<Preview | null>(null);
const problem = ref<string | null>(null);
const looking = ref(true);

const request = computed<TransferRequest>(() => ({
  legs: props.legs,
  source_policy: policy.value,
  verify: verify.value,
  on_conflict: onConflict.value,
  order: order.value,
  save_as: saveAs.value.trim() || null,
}));

onMounted(async () => {
  try {
    preview.value = await transfers.preview(request.value);
  } catch (e) {
    problem.value = String(e);
  } finally {
    looking.value = false;
  }
});

const count = computed(() => props.legs.reduce((n, l) => n + l.names.length, 0));

const LABELS: Record<keyof typeof CHOICES, string> = {
  source_policy: "What happens to the originals",
  verify: "Check each copy by",
  order: "Take files",
  on_conflict: "If the name is already taken there",
};
</script>

<template>
  <Sheet of="drain" :title="props.intent === 'move' ? 'Move files' : 'Copy files'" @dismiss="emit('dismiss')">

    <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
    <p class="set-looking" v-else-if="looking">Working out what would happen…</p>
    <div class="set-what" v-else-if="preview">
      <p class="set-line">
        {{ preview.fresh }} would be sent, {{ bytes(preview.bytes) }} in all.
      </p>
      <p class="set-line" v-if="preview.same_size">
        {{ preview.same_size }} look identical to what is already there and will be checked rather than resent.
      </p>
      <p class="set-line" v-if="preview.clashes">
        {{ preview.clashes }} have a different file of the same name waiting for them.
      </p>
      <p class="set-line" v-if="preview.too_recent">
        {{ preview.too_recent }} were written too recently to touch yet.
      </p>
      <Notice tone="hold" v-if="preview.overlapping.length">
        One side is inside the other. That is refused rather than half-done.
      </Notice>
      <Notice tone="hold" v-else-if="preview.removes_originals">
        Each original is deleted only once its copy has been verified on the far side.
      </Notice>
    </div>

    <div class="set-opts">
      <label v-for="(group, field) in CHOICES" :key="field">
        <span class="set-lbl">{{ LABELS[field] }}</span>
        <select
          :value="field === 'source_policy' ? policy : field === 'verify' ? verify : field === 'order' ? order : onConflict"
          @change="
            field === 'source_policy' ? (policy = ($event.target as HTMLSelectElement).value)
            : field === 'verify' ? (verify = ($event.target as HTMLSelectElement).value)
            : field === 'order' ? (order = ($event.target as HTMLSelectElement).value)
            : (onConflict = ($event.target as HTMLSelectElement).value)
          "
        >
          <option v-for="[value, says] in group" :key="value" :value="value">{{ says }}</option>
        </select>
      </label>
      <label>
        <span class="set-lbl">Save this pair as</span>
        <input v-model="saveAs" placeholder="leave blank to skip" />
      </label>
    </div>

    <div class="set-feet">
      <Button @click="emit('dismiss')">Not now</Button>
      <Button
        look="primary"
        :disabled="looking || !!preview?.overlapping.length"
        @click="emit('go', request)"
      >{{ props.intent === "move" ? "Move them" : "Copy them" }}</Button>
    </div>
  </Sheet>
</template>

<style scoped>.set-looking { font-size: var(--small); color: var(--text-quiet); margin: 0; }
.set-what { display: flex; flex-direction: column; gap: var(--s2); }
.set-line { font-size: var(--small); color: var(--text-quiet); margin: 0; line-height: 1.5; }

.set-opts { display: flex; flex-direction: column; gap: var(--s2); margin-top: var(--s4); }
.set-opts label { display: grid; grid-template-columns: 116px 1fr; gap: var(--s3); align-items: center; }
.set-lbl { font-size: var(--fine); color: var(--text-faint); }
.set-opts select, .set-opts input {
  font: inherit;
  font-size: var(--small);
  background: none;
  color: var(--text);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius);
  padding: 5px 7px;
  min-width: 0;
}
.set-feet { display: flex; justify-content: flex-end; gap: var(--s2); margin-top: var(--s5); }
</style>
