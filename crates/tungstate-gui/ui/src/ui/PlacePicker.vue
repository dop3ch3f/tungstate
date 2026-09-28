<!-- Choose a place: somewhere on this Mac or on any connection, the one picker
     every section uses. With `browse`, a folder inside the place can be opened
     and chosen; without it, choosing a place is the answer, for a caller that
     browses on its own (a Transfer pane). -->
<script setup lang="ts">
import { computed, onMounted, ref, shallowRef } from "vue";
import { folders, transfers } from "../engine/commands";
import type { Entry, Place } from "../engine/types";
import { useConnections } from "../state/useConnections";
import { useNav } from "../nav";
import { latest } from "../lib/latest";
import { kindOf } from "../lib/kinds";
import Sheet from "./Sheet.vue";
import Button from "./Button.vue";
import Notice from "./Notice.vue";
import type { Section } from "../lib/icons";

const props = withDefaults(
  defineProps<{ title?: string; of?: Section; browse?: boolean; choose?: string }>(),
  { title: "Choose a place", of: "drain", browse: true, choose: "Use this folder" },
);
const emit = defineEmits<{ dismiss: []; chosen: [location: string] }>();

const c = useConnections();
const nav = useNav();
const local = shallowRef<Place[]>([]);
const here = ref<string | null>(null);
const folders_ = shallowRef<Entry[]>([]);
const parent = ref<string | null>(null);
const problem = ref<string | null>(null);
const loading = ref(false);
const reads = latest();

onMounted(async () => {
  void c.load();
  try {
    // Connections are listed from the store, with where each points.
    local.value = (await transfers.places()).filter((p) => !p.path.endsWith(":"));
  } catch (e) {
    problem.value = String(e);
  }
});

async function open(to: string) {
  if (!props.browse) {
    emit("chosen", to);
    return;
  }
  loading.value = true;
  problem.value = null;
  const ticket = reads.take();
  try {
    const listing = await transfers.browse(to);
    if (reads.stale(ticket)) return;
    here.value = listing.path;
    parent.value = listing.parent;
    folders_.value = listing.entries.filter((e) => e.is_dir);
  } catch (e) {
    if (!reads.stale(ticket)) problem.value = String(e);
  } finally {
    if (!reads.stale(ticket)) loading.value = false;
  }
}

async function another() {
  const picked = await folders.pick();
  if (picked) await open(picked);
}

function setUp() {
  emit("dismiss");
  nav.go("connections");
}

/** Which entry on the left the open folder is inside, to mark it. */
const inside = computed(() => {
  const at = here.value;
  if (!at) return null;
  const all = [...local.value.map((p) => p.path), ...c.all.value.map((x) => `${x.name}:`)];
  return all.filter((p) => at === p || at.startsWith(p.endsWith("/") || p.endsWith(":") ? p : `${p}/`))
    .sort((a, b) => b.length - a.length)[0] ?? null;
});
</script>

<template>
  <Sheet wide :of="props.of" :title="props.title" @dismiss="emit('dismiss')">
    <div class="pp" :class="{ 'pp-one': !props.browse }">
      <nav class="pp-places">
        <h3>On this Mac</h3>
        <button
          v-for="p in local"
          :key="p.path"
          class="pp-place"
          :class="{ 'pp-on': inside === p.path }"
          @click="open(p.path)"
        >{{ p.label }}</button>
        <button class="pp-place pp-more" @click="another()">Another folder…</button>

        <h3>Connections</h3>
        <button
          v-for="x in c.all.value"
          :key="x.name"
          class="pp-place"
          :class="{ 'pp-on': inside === x.name + ':' }"
          @click="open(x.name + ':')"
        >
          <span>
            <i class="pp-dot" :class="x.last_check ? (x.last_check.ok ? 'pp-ok' : 'pp-bad') : 'pp-never'"></i>{{ x.name }}
          </span>
          <small v-if="x.last_check && !x.last_check.ok" class="pp-fail">Last check did not work</small>
          <small v-else>{{ kindOf(x.scheme).tag }} · {{ x.place }}</small>
        </button>
        <p class="pp-none" v-if="c.loaded.value && !c.all.value.length">None yet.</p>
        <button class="pp-place pp-more" @click="setUp()">Set up a connection…</button>
      </nav>

      <section class="pp-folder" v-if="props.browse">
        <p class="pp-quiet" v-if="!here && !loading">Choose a place on the left, then the folder inside it.</p>
        <template v-else>
          <div class="pp-at">
            <Button :disabled="!parent" @click="parent && open(parent)" title="Up one folder">↑</Button>
            <span class="path pp-path">{{ here }}</span>
          </div>
          <Notice tone="bad" v-if="problem">{{ problem }}</Notice>
          <ul class="pp-list" :aria-busy="loading || undefined">
            <li v-for="f in folders_" :key="f.path">
              <button class="pp-sub" @click="open(f.path)">{{ f.name }}</button>
            </li>
            <li class="pp-quiet" v-if="!loading && !folders_.length && !problem">No folders inside. This one can still be used.</li>
          </ul>
        </template>
      </section>
    </div>
    <Notice tone="bad" v-if="problem && !props.browse">{{ problem }}</Notice>

    <div class="pp-foot">
      <Button @click="emit('dismiss')">Cancel</Button>
      <Button v-if="props.browse" look="primary" :disabled="!here || loading" @click="here && emit('chosen', here)">
        {{ props.choose }}
      </Button>
    </div>
  </Sheet>
</template>

<style scoped>
.pp { display: grid; grid-template-columns: 220px minmax(0, 1fr); gap: var(--s4); min-height: 320px; }
.pp-one { grid-template-columns: minmax(0, 1fr); min-height: 0; }
.pp-places { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.pp-places h3 { font-size: var(--fine); font-weight: 600; color: var(--text-faint); margin: var(--s3) 0 var(--s1); }
.pp-places h3:first-child { margin-top: 0; }
.pp-place {
  font: inherit;
  font-size: var(--small);
  text-align: left;
  display: flex;
  flex-direction: column;
  background: none;
  border: var(--bw) solid transparent;
  border-radius: var(--radius);
  color: var(--text);
  padding: 4px var(--s2);
  cursor: pointer;
  min-width: 0;
}
.pp-place small { font-size: var(--fine); color: var(--text-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pp-place:hover { background: var(--surface-hover); }
.pp-on { background: var(--chosen); color: var(--chosen-ink); border-color: var(--chosen-edge); }
.pp-more { color: var(--text-quiet); }
/* How the connection's last check went, as on its row in Connections. */
.pp-dot { display: inline-block; width: 6px; height: 6px; border-radius: 50%; margin: 0 6px 1px 0; }
.pp-ok { background: var(--ok); }
.pp-bad { background: var(--bad); }
.pp-never { background: var(--text-faint); opacity: 0.5; }
.pp-place small.pp-fail { color: var(--bad); }
.pp-none { font-size: var(--fine); color: var(--text-faint); margin: 0 var(--s2); }
.pp-folder { display: flex; flex-direction: column; gap: var(--s2); min-width: 0; border-left: var(--bw) solid var(--rule); padding-left: var(--s4); }
.pp-at { display: flex; align-items: center; gap: var(--s2); min-width: 0; }
.pp-path { font-size: var(--fine); color: var(--text-quiet); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; word-break: normal; }
.pp-list { list-style: none; margin: 0; padding: 0; overflow-y: auto; max-height: 300px; }
.pp-list[aria-busy] { opacity: 0.55; }
.pp-sub {
  font: inherit;
  font-size: var(--small);
  width: 100%;
  text-align: left;
  background: none;
  border: none;
  border-bottom: var(--bw) solid var(--rule);
  color: var(--text);
  padding: 6px var(--s1);
  cursor: pointer;
}
.pp-sub:hover { background: var(--surface-hover); }
.pp-quiet { font-size: var(--fine); color: var(--text-faint); margin: var(--s2) 0; }
.pp-foot { display: flex; justify-content: flex-end; gap: var(--s2); margin-top: var(--s4); }
</style>
