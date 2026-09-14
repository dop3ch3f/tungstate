<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api, bytes, type ArchiveView } from "../api";

const emit = defineEmits<{ changed: [] }>();

const list = ref<ArchiveView[]>([]);
const error = ref("");
const note = ref("");
const busy = ref("");
const confirmingReset = ref(false);
const confirmingForget = ref<ArchiveView | null>(null);

async function refresh() {
  try {
    list.value = await api.archives();
  } catch (e) {
    error.value = String(e);
  }
}
onMounted(refresh);

function when(ms: number): string {
  return ms ? new Date(ms).toLocaleString() : "unknown";
}

async function reset() {
  busy.value = "#reset";
  error.value = "";
  note.value = "";
  try {
    const name = await api.resetStorage();
    note.value = `Everything was archived as ${name}, and this is now an empty slate. Nothing was deleted — bring it back from the list below whenever you like.`;
    confirmingReset.value = false;
    await refresh();
    emit("changed");
  } catch (e) {
    error.value = String(e);
    confirmingReset.value = false;
  } finally {
    busy.value = "";
  }
}

async function restore(archive: ArchiveView) {
  busy.value = archive.name;
  error.value = "";
  note.value = "";
  try {
    const putAside = await api.restoreArchive(archive.name);
    note.value = `Restored ${archive.name}. What was here is archived as ${putAside}, so this is reversible too. Connection passwords are still in your keychain and will be found again by name.`;
    await refresh();
    emit("changed");
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function forget(archive: ArchiveView) {
  busy.value = archive.name;
  error.value = "";
  try {
    await api.forgetArchive(archive.name);
    note.value = `Deleted ${archive.name}.`;
    confirmingForget.value = null;
    await refresh();
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function exportTo() {
  error.value = "";
  note.value = "";
  const path = await api.pickSaveFile("tungstate-export.json");
  if (!path) return;
  busy.value = "#export";
  try {
    await api.exportStorage(path);
    note.value = `Written to ${path}. It carries no passwords — those stay in your keychain.`;
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function importFrom() {
  error.value = "";
  note.value = "";
  const path = await api.pickOpenFile();
  if (!path) return;
  busy.value = "#import";
  try {
    const rows = await api.importStorage(path);
    note.value = `Read ${rows} rows from ${path}. Re-enter any connection passwords from the Connections tab.`;
    emit("changed");
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}
</script>

<template>
  <div class="sheet">
    <h1>Storage</h1>
    <p class="sub">
      What tungstate remembers: your connections, your links, and the record of every file it
      has moved. None of this is your files — clearing it changes nothing on disk or on the
      NAS, it only forgets what happened.
    </p>

    <div v-if="error" class="notice bad">{{ error }}</div>
    <div v-if="note" class="notice">{{ note }}</div>

    <div class="go" style="margin-bottom:16px">
      <button class="btn" :disabled="busy === '#export'" @click="exportTo">Export…</button>
      <button class="btn" :disabled="busy === '#import'" @click="importFrom">Import…</button>
      <button class="btn" :disabled="busy === '#reset'" @click="confirmingReset = true">
        Start fresh…
      </button>
    </div>

    <h2>Archives</h2>
    <p class="sub">
      Every time you start fresh or restore, what was here is written out first. Nothing is
      ever thrown away unless you say so on this screen.
    </p>

    <div v-if="!list.length" class="empty">
      <strong>Nothing archived yet.</strong>
      <span>Start fresh, and what you have now appears here, ready to come back.</span>
    </div>

    <table v-else class="ledger">
      <thead>
        <tr>
          <th>When</th>
          <th>Holds</th>
          <th class="num">Size</th>
          <th class="actions"></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="a in list" :key="a.name">
          <td>
            {{ when(a.archived_at) }}
            <div class="detail">{{ a.name }}</div>
          </td>
          <td>
            <template v-if="a.operations !== null">
              <div class="detail">{{ a.links }} link(s), {{ a.connections }} connection(s)</div>
              <div class="detail">{{ a.operations }} operation(s) recorded</div>
            </template>
            <!-- Shown rather than hidden: an archive that will not open is
                 exactly what somebody needs to know about. -->
            <span v-else class="detail warn">this one cannot be read</span>
          </td>
          <td class="num">{{ bytes(a.size) }}</td>
          <td class="actions">
            <button class="btn quiet small" :disabled="busy === a.name || a.operations === null"
                    @click="restore(a)">Restore</button>
            <button class="btn quiet small" :disabled="busy === a.name"
                    @click="confirmingForget = a">Delete</button>
          </td>
        </tr>
      </tbody>
    </table>

    <div v-if="confirmingReset" class="veil" @click.self="confirmingReset = false">
      <div class="modal" role="dialog" aria-modal="true">
        <div class="cap">
          <h2>Start fresh?</h2>
          <p class="why">
            Your connections, links and history are written out to an archive first, and this
            becomes an empty slate. <b>No file is moved or deleted anywhere</b> — not on this
            machine and not on the NAS. You can bring the archive back from this screen at any
            time, and your saved passwords stay in the keychain either way.
          </p>
        </div>
        <div class="feet">
          <span class="spacer"></span>
          <button class="btn" @click="confirmingReset = false">Cancel</button>
          <button class="btn primary" :disabled="busy === '#reset'" @click="reset">
            Archive and start fresh
          </button>
        </div>
      </div>
    </div>

    <div v-if="confirmingForget" class="veil" @click.self="confirmingForget = null">
      <div class="modal" role="dialog" aria-modal="true">
        <div class="cap">
          <h2>Delete {{ confirmingForget.name }}?</h2>
          <p class="why">
            This is the only thing on this screen that loses something. The archive is removed
            for good and cannot be restored afterwards.
          </p>
        </div>
        <div class="feet">
          <span class="spacer"></span>
          <button class="btn" @click="confirmingForget = null">Keep it</button>
          <button class="btn primary" :disabled="busy === confirmingForget.name"
                  @click="forget(confirmingForget)">Delete for good</button>
        </div>
      </div>
    </div>
  </div>
</template>
