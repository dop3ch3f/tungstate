<script setup lang="ts">
import { ref, computed } from "vue";
import { api, bytes, type Op, type Link } from "../api";

const props = defineProps<{ links: Link[] }>();

const target = ref("");
const ops = ref<Op[] | null>(null);
const error = ref("");
const searching = ref(false);

/** A BLAKE3 digest is 64 hex characters, and no real path looks like one, so
 *  the shape of what you typed decides how it is read. Same rule as the
 *  command line, which is the point: one answer, however you ask. */
const isHash = computed(
  () => target.value.length === 64 && /^[0-9a-fA-F]+$/.test(target.value),
);

async function find() {
  const asked = target.value.trim();
  if (!asked) return;
  searching.value = true;
  error.value = "";
  ops.value = null;
  try {
    // `whereis` answers "where did it end up", history answers "everything
    // that ever happened to it". Asking both and showing the fuller one is
    // kinder than making you pick.
    ops.value = isHash.value ? await api.whereis(asked) : await api.history(asked);
  } catch (e) {
    error.value = String(e);
  } finally {
    searching.value = false;
  }
}

/** What was set aside at a link's destination, rather than moved. */
const quarantine = ref<Record<string, string[]>>({});
const looking = ref("");

async function inspect(name: string) {
  looking.value = name;
  error.value = "";
  try {
    quarantine.value = { ...quarantine.value, [name]: await api.quarantined(name) };
  } catch (e) {
    error.value = String(e);
  } finally {
    looking.value = "";
  }
}

function when(ms: number): string {
  return new Date(ms).toLocaleString();
}

function verdict(op: Op): string {
  switch (op.status) {
    case "committed": return "done";
    case "intended": return "interrupted";
    case "failed": return "failed";
    default: return op.status;
  }
}
</script>

<template>
  <div class="sheet">
    <h1>Find a file</h1>
    <p class="sub">
      Where something went, and everything that ever happened to it. Give it a path, or a
      fingerprint if you have one — a file that was moved and renamed is still findable by
      what it contains.
    </p>

    <div class="opt">
      <label for="find">Path or fingerprint</label>
      <input id="find" v-model="target" autocomplete="off" spellcheck="false"
             placeholder="/Users/you/Movies/holiday.mp4" @keyup.enter="find" />
      <span class="note">
        {{ isHash ? "Reading this as a fingerprint." : "Reading this as a path." }}
      </span>
    </div>
    <div class="go">
      <button class="btn primary" :disabled="searching || !target.trim()" @click="find">
        {{ searching ? "Looking…" : "Find it" }}
      </button>
    </div>

    <div v-if="error" class="notice bad">{{ error }}</div>

    <div v-if="ops && !ops.length" class="empty">
      <strong>Nothing recorded for that.</strong>
      <span>
        Only what tungstate moved is in here. A file it never touched has no history, which
        is not the same as the file not existing.
      </span>
    </div>

    <table v-else-if="ops" class="ledger">
      <thead>
        <tr>
          <th>When</th>
          <th>What</th>
          <th>From</th>
          <th>To</th>
          <th class="num">Size</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="op in ops" :key="op.id">
          <td class="detail">{{ when(op.started_at) }}</td>
          <td>
            <span class="state" :class="op.status === 'committed' ? 'ok' : op.status">
              {{ verdict(op) }}
            </span>
            <div v-if="op.link" class="detail">{{ op.link }}</div>
            <div v-if="op.note" class="detail">{{ op.note }}</div>
          </td>
          <td class="addr">{{ op.source ?? "—" }}</td>
          <td class="addr">{{ op.destination ?? "—" }}</td>
          <td class="num">{{ op.size === null ? "—" : bytes(op.size) }}</td>
        </tr>
      </tbody>
    </table>

    <template v-if="props.links.length">
      <h2 style="margin-top:26px">Set aside</h2>
      <p class="sub">
        When a different file already holds the name, tungstate parks the new one rather than
        choosing for you. Nothing here was lost, and nothing here was overwritten.
      </p>
      <table class="ledger">
        <tbody>
          <tr v-for="l in props.links" :key="l.name">
            <td><b>{{ l.name }}</b><div class="detail">{{ l.destination }}</div></td>
            <td>
              <template v-if="quarantine[l.name]">
                <span v-if="!quarantine[l.name].length" class="detail">nothing set aside</span>
                <div v-for="path in quarantine[l.name]" :key="path" class="detail">{{ path }}</div>
              </template>
              <span v-else class="detail">not looked at</span>
            </td>
            <td class="actions">
              <button class="btn quiet small" :disabled="looking === l.name"
                      @click="inspect(l.name)">Look</button>
            </td>
          </tr>
        </tbody>
      </table>
    </template>
  </div>
</template>
