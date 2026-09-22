<!-- A saved pair made without moving anything, the way `link add` works on
     the command line. Nothing moves until it is run. -->
<script setup lang="ts">
import { computed, ref } from "vue";
import { links } from "../../engine/commands";
import { CHOICES, type NewLink, type Place } from "../../engine/types";
import Sheet from "../../ui/Sheet.vue";
import Field from "../../ui/Field.vue";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";

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
  <Sheet wide @dismiss="emit('dismiss')">
    <h2 class="pf-title">A saved pair</h2>
    <p class="pf-why">
      Two folders and what should happen between them. Nothing moves until you run it. A
      connection is written <span class="mono">name:folder</span>, as on the command line.
    </p>

    <Field label="Called"><input v-model="form.name" autocomplete="off" placeholder="laptop to nas" /></Field>
    <div class="pf-grid">
      <Field label="From"><input v-model="form.source" list="pf-places" spellcheck="false" placeholder="/Users/you/Movies" /></Field>
      <Field label="To"><input v-model="form.destination" list="pf-places" spellcheck="false" placeholder="nas:inbox" /></Field>
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
      <Field label="Wait after a file is written" note="Seconds a file must sit unchanged before it is taken.">
        <input type="number" min="0" v-model.number="form.cooldown_secs" />
      </Field>
    </div>

    <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
    <div class="pf-foot">
      <Button @click="emit('dismiss')">Cancel</Button>
      <Button look="primary" :disabled="!ready || saving" @click="save()">{{ saving ? "Saving…" : "Save the pair" }}</Button>
    </div>
  </Sheet>
</template>

<style scoped>
.pf-title { font-size: var(--body); font-weight: 700; margin: 0; }
.pf-why { font-size: var(--small); color: var(--text-quiet); margin: var(--s2) 0 var(--s4); line-height: 1.5; }
.pf-grid { display: grid; grid-template-columns: 1fr 1fr; gap: var(--s3); margin-top: var(--s3); }
.pf-foot { display: flex; justify-content: flex-end; gap: var(--s2); margin-top: var(--s5); }
</style>
