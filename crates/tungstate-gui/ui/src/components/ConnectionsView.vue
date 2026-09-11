<script setup lang="ts">
import { ref } from "vue";
import ConnectionModal from "./ConnectionModal.vue";
import { api, type Connection } from "../api";

const props = defineProps<{ connections: Connection[] }>();
const emit = defineEmits<{ changed: []; browse: [location: string] }>();

/** Per-row, keyed by name: several rows can be mid-test at once. */
type Probe = { state: "waiting" | "ok" | "failed"; detail: string };
const probes = ref<Record<string, Probe>>({});

const adding = ref(false);
const editing = ref<Connection | null>(null);
const changing = ref<Connection | null>(null);
const newSecret = ref("");
const busy = ref("");
const error = ref("");

async function test(name: string) {
  probes.value = { ...probes.value, [name]: { state: "waiting", detail: "Reaching it…" } };
  try {
    const found = await api.testConnection(name);
    probes.value = {
      ...probes.value,
      [name]: {
        state: "ok",
        // The root travels with the count for the same reason the CLI prints
        // both: "18 entries" is reassuring and useless when the 18 are the
        // server's own /bin.
        detail: `${found.root} holds ${found.entries} ${found.entries === 1 ? "entry" : "entries"}`,
      },
    };
  } catch (e) {
    probes.value = { ...probes.value, [name]: { state: "failed", detail: String(e) } };
  }
}

async function forget(c: Connection) {
  busy.value = c.name;
  error.value = "";
  try {
    await api.removeConnection(c.name);
    emit("changed");
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function savePassword() {
  if (!changing.value) return;
  const target = changing.value.name;
  busy.value = target;
  error.value = "";
  try {
    await api.setConnectionPassword(target, newSecret.value);
    changing.value = null;
    // Held no longer than the call that needed it.
    newSecret.value = "";
    // Proving the new one works is the whole reason anyone opened this.
    await test(target);
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

function saved() {
  adding.value = false;
  editing.value = null;
  emit("changed");
}
</script>

<template>
  <div class="sheet">
    <div style="display:flex; align-items:baseline; justify-content:space-between; gap:12px">
      <h1>Connections</h1>
      <div class="go">
        <button class="btn primary" @click="adding = true">Add a connection</button>
      </div>
    </div>
    <p class="sub">
      Places files can go that are not this machine. A connection works whether or not the
      volume is mounted, which is the point of it — the NAS is reachable here even when Finder
      cannot see it.
    </p>

    <div v-if="error" class="notice bad">{{ error }}</div>

    <div v-if="!props.connections.length" class="empty">
      <strong>No connections yet.</strong>
      <span>Add one and it appears in each pane's “Go to…” list, ready to browse and drain into.</span>
    </div>

    <table v-else class="ledger">
      <thead>
        <tr>
          <th>Name</th>
          <th>Where</th>
          <th>Folder</th>
          <th>Last check</th>
          <th class="actions"></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="c in props.connections" :key="c.name">
          <td>
            <b>{{ c.name }}</b>
            <div class="detail">
              {{ c.scheme }}<template v-if="!c.encrypted"> · sends your password in the clear</template>
            </div>
          </td>
          <td class="path">
            <template v-if="c.host">{{ c.host }}<template v-if="c.port">:{{ c.port }}</template></template>
            <template v-else>this machine</template>
            <div v-if="c.username" class="detail">as {{ c.username }}</div>
          </td>
          <td class="path">{{ c.root || "/" }}</td>
          <td>
            <span v-if="probes[c.name]" class="state" :class="probes[c.name].state">
              {{ probes[c.name].state === "waiting" ? "checking" :
                 probes[c.name].state === "ok" ? "reachable" : "unreachable" }}
            </span>
            <span v-else class="state waiting">not checked</span>
            <div v-if="probes[c.name]" class="detail">{{ probes[c.name].detail }}</div>
          </td>
          <td class="actions">
            <button class="btn quiet small" @click="test(c.name)">Test</button>
            <button class="btn quiet small" @click="emit('browse', `${c.name}:`)">Browse</button>
            <button class="btn quiet small" @click="editing = c">Edit</button>
            <button v-if="c.networked" class="btn quiet small"
                    @click="changing = c; newSecret = ''">Password</button>
            <button class="btn quiet small" :disabled="busy === c.name" @click="forget(c)">Forget</button>
          </td>
        </tr>
      </tbody>
    </table>

    <ConnectionModal v-if="adding || editing" :editing="editing"
                     @cancel="adding = false; editing = null" @saved="saved" />

    <!-- Its own dialog rather than a field in the edit form. A stored password
         cannot be read back to prefill, so a blank box inside a form that
         saves every other field would have to mean two different things. -->
    <div v-if="changing" class="veil" @click.self="changing = null; newSecret = ''">
      <div class="modal" role="dialog" aria-modal="true">
        <div class="cap">
          <h2>Password for {{ changing.name }}</h2>
          <p class="why">
            Replaces whatever is stored. Use this when a sign-in is refused: the old password is
            not shown anywhere, so there is nothing to compare against — type the current one
            again if you are unsure.
          </p>
        </div>
        <div class="body">
          <div class="opt">
            <label for="newpass">New password</label>
            <input id="newpass" type="password" v-model="newSecret" autocomplete="off"
                   @keyup.enter="savePassword" />
            <span class="note">Leave it blank to sign in without one.</span>
          </div>
        </div>
        <div class="feet">
          <span class="spacer"></span>
          <button class="btn" @click="changing = null; newSecret = ''">Cancel</button>
          <button class="btn primary" :disabled="busy === changing.name" @click="savePassword">
            Save and test
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
