<!-- A saved pair made without moving anything, the way `link add` works on
     the command line. Nothing moves until it is run. -->
<script setup lang="ts">
import { computed, ref } from "vue";
import { links } from "../../engine/commands";
import { CHOICES, type NewLink, type Place } from "../../engine/types";
import Sheet from "../../ui/Sheet.vue";
import Field from "../../ui/Field.vue";
import Button from "../../ui/Button.vue";
import ActionBar from "../../ui/ActionBar.vue";
import Notice from "../../ui/Notice.vue";
import PlacePicker from "../../ui/PlacePicker.vue";

const props = defineProps<{ places: Place[] }>();
const emit = defineEmits<{ dismiss: []; saved: [name: string] }>();

const form = ref<NewLink>({
  name: "",
  source: "",
  destination: "",
  source_policy: "delete",
  verify: "hash",
  order: "largest-first",
  on_conflict: "quarantine",
  cooldown_secs: 30,
});
const saving = ref(false);
/** Which end the place picker is choosing, when it is open. */
const choosing = ref<"source" | "destination" | null>(null);
/** The rarely-changed wait, out of the way until asked for. */
const more = ref(false);
const problem = ref<string | null>(null);
const ready = computed(() => !!(form.value.name.trim() && form.value.source.trim() && form.value.destination.trim()));

const LABELS: Record<keyof typeof CHOICES, string> = {
  source_policy: "What happens to the originals",
  verify: "Check each copy by",
  order: "Take files",
  on_conflict: "If the name is already taken there",
};

async function save() {
  saving.value = true;
  problem.value = null;
  try {
    await links.create({ ...form.value, name: form.value.name.trim() });
    emit("saved", form.value.name.trim());
  } catch (e) {
    problem.value = String(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <Sheet wide of="drain" title="A saved pair" @dismiss="emit('dismiss')">
    <p class="pf-why">
      Two folders and what happens between them. Nothing moves until you run it.
    </p>

    <Field label="Called"><input v-model="form.name" autocomplete="off" placeholder="laptop to nas" /></Field>
    <div class="pf-grid">
      <Field label="From">
        <span class="pf-place">
          <input v-model="form.source" list="pf-places" spellcheck="false" placeholder="/Users/you/Movies" />
          <Button @click="choosing = 'source'">Choose…</Button>
        </span>
      </Field>
      <Field label="To">
        <span class="pf-place">
          <input v-model="form.destination" list="pf-places" spellcheck="false" placeholder="nas:inbox" />
          <Button @click="choosing = 'destination'">Choose…</Button>
        </span>
      </Field>
    </div>
    <datalist id="pf-places">
      <option v-for="p in props.places" :key="p.path" :value="p.path">{{ p.label }}</option>
    </datalist>

    <div class="pf-grid">
      <Field v-for="(group, field) in CHOICES" :key="field" :label="LABELS[field]">
        <select v-model="form[field]">
          <option v-for="[value, says] in group" :key="value" :value="value">{{ says }}</option>
        </select>
      </Field>
    </div>
    <Button look="link" class="pf-more" @click="more = !more">{{ more ? "Fewer options" : "More options…" }}</Button>
    <div class="pf-grid" v-if="more">
      <Field label="Wait after a file is written" note="Seconds a file must sit unchanged before it is taken.">
        <input type="number" min="0" v-model.number="form.cooldown_secs" />
      </Field>
    </div>
    <PlacePicker
      v-if="choosing"
      of="drain"
      :title="choosing === 'source' ? 'Take files from' : 'Send files to'"
      @dismiss="choosing = null"
      @chosen="(at) => { form[choosing!] = at; choosing = null }"
    />

    <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
    <ActionBar pinned>
      <Button @click="emit('dismiss')">Cancel</Button>
      <Button look="primary" :disabled="!ready" :busy="saving" @click="save()">Save the pair</Button>
    </ActionBar>
  </Sheet>
</template>

<style scoped>.pf-why { font-size: var(--small); color: var(--text-quiet); margin: var(--s2) 0 var(--s4); line-height: 1.5; }
.pf-grid { display: grid; grid-template-columns: 1fr 1fr; gap: var(--s3); margin-top: var(--s3); }
.pf-place { display: flex; gap: var(--s2); }
.pf-place input { flex: 1; min-width: 0; }
.pf-more { margin-top: var(--s3); }
</style>
