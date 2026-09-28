<!-- Saved pairs: where from, where to, and the six settings in plain words. -->
<script setup lang="ts">
import { onMounted, ref, shallowRef } from "vue";
import { history, links } from "../../engine/commands";
import { CHOICES, type Link, type Place, type Preview } from "../../engine/types";
import { bytes, duration } from "../../lib/format";
import { ask } from "../../ui/useDialog";
import Button from "../../ui/Button.vue";
import RowMenu from "../../ui/RowMenu.vue";
import Notice from "../../ui/Notice.vue";
import Empty from "../../ui/Empty.vue";
import Sheet from "../../ui/Sheet.vue";
import PairForm from "./PairForm.vue";
import { useBusy } from "../../state/useBusy";
import { useTransfer } from "../../state/useTransfer";

const props = defineProps<{ places: Place[] }>();
const emit = defineEmits<{ ran: [] }>();
const adding = ref(false);
const said = ref<string | null>(null);
const looking = ref<{ name: string; preview: Preview | null } | null>(null);
/** What each pair set aside, fetched when asked for. */
const setAside = ref<Record<string, string[]>>({});

async function saved(name: string) {
  adding.value = false;
  said.value = `Saved ${name}. Nothing has moved; run it when you are ready.`;
  await load();
}

async function preview(name: string) {
  looking.value = { name, preview: null };
  try {
    looking.value = { name, preview: await links.preview(name) };
  } catch (e) {
    looking.value = null;
    problem.value = String(e);
  }
}

async function showSetAside(name: string) {
  try {
    setAside.value = { ...setAside.value, [name]: await history.quarantined(name) };
  } catch (e) {
    problem.value = String(e);
  }
}

/** One sentence per count, each only when it is not zero. Built as a string
 *  because adjacent template fragments lose the space between them. */
function summary(p: Preview): string {
  const n = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;
  return [
    `${n(p.fresh, "file will move", "files will move")}, ${bytes(p.bytes)}.`,
    p.same_size ? `${n(p.same_size, "may", "may")} already be there; ${p.same_size === 1 ? "it is" : "they are"} checked rather than resent.` : "",
    p.clashes ? `${n(p.clashes, "has", "have")} a different file of that name waiting.` : "",
    p.too_recent ? `${n(p.too_recent, "is", "are")} too recent to take yet.` : "",
  ].filter(Boolean).join(" ");
}

const leafOf = (path: string) => path.replace(/[/:]+$/, "").split(/[/:]/).pop() || path;
/** Where each previewed file would land, said once above the list. */
const into = (p: Preview) => {
  const first = p.items[0]?.path ?? "";
  return first.slice(0, Math.max(first.lastIndexOf("/"), first.lastIndexOf(":")) + 1);
};
const WORD: Record<string, string> = { move: "will move", check: "may be there, checked", clash: "name taken", hold: "too recent" };
const all = shallowRef<Link[]>([]);
const problem = ref<string | null>(null);

async function load() {
  try {
    all.value = await links.list();
  } catch (e) {
    problem.value = String(e);
  }
}
onMounted(load);

/** The prose for a stored token, from the same table the dialog offers. */
function says(field: keyof typeof CHOICES, token: string): string {
  const found = (CHOICES[field] as readonly (readonly [string, string])[]).find(([v]) => v === token);
  return found ? found[1] : token;
}

// A second Run while the first is being handed over would queue the pair
// twice, so each link's actions wait for the one before.
const doing = useBusy();
const transfer = useTransfer();

const run = (name: string) =>
  doing.run(name, async () => {
    try {
      const accepted = await links.run(name);
      transfer.accepted(accepted);
      if (accepted.started) emit("ran");
    } catch (e) {
      problem.value = String(e);
    }
  });

async function remove(link: Link) {
  const answer = await ask({
    title: `Remove ${link.name}?`,
    why: "If this pair has moved anything, it is retired rather than deleted: its history stays, so a file can still be traced. If it has never run, it goes for good.",
    choices: [
      { id: "no", label: "Keep it" },
      { id: "yes", label: "Remove", look: "danger" },
    ],
  });
  if (answer.id !== "yes") return;
  await doing.run(link.name, async () => {
    try {
      await links.remove(link.name);
      await load();
    } catch (e) {
      problem.value = String(e);
    }
  });
}
</script>

<template>
  <div class="lk-wrap">
    <div class="lk-top">
      <span></span>
      <Button look="primary" v-if="all.length" @click="adding = true">Add a saved pair</Button>
    </div>
    <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
    <Notice v-if="said">{{ said }}</Notice>
    <Empty v-if="!all.length" art="no-pairs" line="Save a transfer you repeat, like Movies to the NAS, and run it again with one press.">
      <Button look="primary" @click="adding = true">Add a saved pair</Button>
    </Empty>
    <div class="lk-row" v-for="link in all" :key="link.name">
      <div class="lk-who">
        <span class="lk-name">{{ link.name }}</span>
        <span class="lk-ends" :title="`${link.source} → ${link.destination}`">{{ leafOf(link.source) }} → {{ leafOf(link.destination) }}</span>
      </div>
      <!-- The one setting that matters most first; the rest on hover. -->
      <ul class="lk-how" :title="[says('source_policy', link.source_policy), says('verify', link.verify), says('order', link.order), says('on_conflict', link.on_conflict)].join('\n')">
        <li>{{ link.source_policy === "delete" ? "Moves" : "Copies" }}, checking each file<template v-if="link.cooldown_secs"> · waits {{ duration(link.cooldown_secs) }}</template></li>
        <li v-if="setAside[link.name]" class="lk-aside">
          <template v-if="!setAside[link.name].length">nothing set aside</template>
          <template v-else>set aside: <span class="path" v-for="p in setAside[link.name]" :key="p">{{ p }}</span></template>
        </li>
      </ul>
      <div class="lk-do">
        <Button look="primary" :busy="doing.busy(link.name)" @click="run(link.name)">Run</Button>
        <Button @click="preview(link.name)">Preview</Button>
        <RowMenu
          :label="`More for ${link.name}`"
          :items="[
            { id: 'aside', label: 'What it set aside' },
            { id: 'remove', label: 'Remove', danger: true, busy: doing.busy(link.name) },
          ]"
          @pick="(id) => (id === 'aside' ? showSetAside(link.name) : remove(link))"
        />
      </div>
    </div>

    <PairForm v-if="adding" :places="props.places" @dismiss="adding = false" @saved="saved" />

    <Sheet v-if="looking" wide of="drain" :title="`Preview ${looking.name}`" @dismiss="looking = null">
      <p class="lk-lede">Nothing has happened yet.</p>
      <p class="lk-lede" v-if="!looking.preview">Working it out…</p>
      <template v-else>
        <p class="lk-sum">{{ summary(looking.preview) }}</p>
        <p class="lk-lede path" v-if="into(looking.preview)">Into {{ into(looking.preview) }}</p>
        <ul class="lk-items">
          <li v-for="(item, i) in looking.preview.items.slice(0, 60)" :key="item.path + i">
            <span class="path lk-p" :title="item.path">{{ item.path.slice(into(looking.preview).length) || item.path }}</span>
            <span class="lk-w">{{ WORD[item.outcome] ?? item.outcome }}</span>
            <span class="num lk-s">{{ bytes(item.size) }}</span>
          </li>
        </ul>
        <p class="lk-lede" v-if="looking.preview.items.length > 60">and {{ looking.preview.items.length - 60 }} more</p>
        <p class="lk-lede" v-if="looking.preview.removes_originals">Each original is removed only after its copy checks out.</p>
      </template>
      <div class="lk-qf">
        <Button @click="looking = null">Close</Button>
        <Button look="primary" :disabled="!looking.preview" @click="run(looking.name); looking = null">Run it</Button>
      </div>
    </Sheet>
  </div>
</template>

<style scoped>
.lk-wrap { display: flex; flex-direction: column; gap: var(--s3); container-type: inline-size; }
.lk-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) auto;
  gap: var(--s4);
  align-items: start;
  padding: var(--s3) 0;
  border-bottom: 1px solid var(--rule);
}
.lk-who { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.lk-name { font-weight: 600; font-size: var(--small); }
.lk-ends { font-size: var(--small); color: var(--text-quiet); }
/* Narrow: a pair stacks rather than squeezing its two paths into a column
   too thin to hold one. */
@container (max-width: 680px) {
  .lk-row { grid-template-columns: minmax(0, 1fr); gap: var(--s2); }
  .lk-do { justify-content: flex-start; }
}
.lk-how li { white-space: nowrap; }
.lk-how { list-style: none; margin: 0; padding: 0; font-size: var(--fine); color: var(--text-quiet); }
.lk-how li { padding: 1px 0; }
.lk-do { display: flex; gap: var(--s2); flex-wrap: wrap; justify-content: flex-end; }
.lk-top { display: flex; align-items: center; justify-content: space-between; gap: var(--s4); }
.lk-lede { font-size: var(--small); color: var(--text-quiet); margin: 0; }
.lk-aside { color: var(--hold); display: flex; flex-wrap: wrap; gap: var(--s2); }.lk-sum { font-size: var(--small); margin: var(--s3) 0; line-height: 1.5; }
.lk-items { list-style: none; margin: 0 0 var(--s3); padding: 0; max-height: 260px; overflow-y: auto; border-top: 1px solid var(--rule); }
.lk-items li { display: grid; grid-template-columns: minmax(0, 1fr) auto 70px; gap: var(--s3); padding: 4px 0; border-bottom: 1px solid var(--rule); font-size: var(--fine); }
.lk-p { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; word-break: normal; }
.lk-w { color: var(--text-faint); }
.lk-s { text-align: right; color: var(--text-faint); }
.lk-qf { display: flex; justify-content: flex-end; gap: var(--s2); margin-top: var(--s4); }
</style>
