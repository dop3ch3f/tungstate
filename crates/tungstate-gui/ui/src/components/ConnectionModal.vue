<script setup lang="ts">
import { ref, computed } from "vue";
import { api, type Connection, type ConnectionForm } from "../api";

const props = defineProps<{
  /** The connection being edited, or null when adding a new one. */
  editing: Connection | null;
}>();
const emit = defineEmits<{ cancel: []; saved: [name: string] }>();

const name = ref(props.editing?.name ?? "");
const scheme = ref(props.editing?.scheme ?? "ftp");
const host = ref(props.editing?.host ?? "");
const port = ref(props.editing?.port ? String(props.editing.port) : "");
const username = ref(props.editing?.username ?? "");
const root = ref(props.editing?.root ?? "");

// Only on the add path. Changing a password on an existing connection is its
// own button on the row, because it is its own decision: a stored secret
// cannot be read back to show in a form, so a blank field here would have to
// mean "leave it" on edit and "no password" on add.
const secret = ref("");

const saving = ref(false);
const error = ref("");

const local = computed(() => scheme.value === "fs");
const clear = computed(() => scheme.value === "ftp");
const authenticates = computed(() => !local.value);

const ready = computed(() => {
  if (!name.value.trim()) return false;
  // The two-character rule from `parse_end`: one character before a colon is
  // a Windows drive letter, so a one-character connection name could never be
  // written in a link spec.
  if (!/^[A-Za-z0-9_-]{2,}$/.test(name.value.trim())) return false;
  return local.value || !!host.value.trim();
});

const nameProblem = computed(() => {
  const typed = name.value.trim();
  if (!typed || /^[A-Za-z0-9_-]{2,}$/.test(typed)) return "";
  return "Two or more letters, digits, - or _. This is what you type before the colon in a location.";
});

function form(): ConnectionForm {
  const typed = Number(port.value);
  return {
    name: name.value.trim(),
    scheme: scheme.value,
    host: host.value.trim() || null,
    port: port.value.trim() && Number.isFinite(typed) ? typed : null,
    username: username.value.trim() || null,
    root: root.value.trim(),
    // No per-scheme extras on screen yet. Sent so an edit never silently
    // drops what the command line put there.
    options: props.editing?.options ?? {},
  };
}

async function save() {
  saving.value = true;
  error.value = "";
  try {
    if (props.editing) await api.updateConnection(props.editing.name, form());
    else await api.addConnection(form(), authenticates.value ? secret.value : null);
    // Cleared the moment it is no longer needed, so it is not sitting in a
    // component that stays alive behind the dialog.
    secret.value = "";
    emit("saved", name.value.trim());
  } catch (e) {
    error.value = String(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="veil" @click.self="emit('cancel')">
    <div class="modal" role="dialog" aria-modal="true">
      <div class="cap">
        <h2>{{ props.editing ? `Edit ${props.editing.name}` : "Add a connection" }}</h2>
        <p class="why">
          A place files can go that is not this machine. Once it is here you can browse it in a
          pane and move files onto it, whether or not it is mounted in Finder.
        </p>
      </div>

      <div class="body">
        <div class="pair">
          <div class="opt">
            <label for="cname">Name</label>
            <!-- Disabled rather than hidden on edit: it is the answer to
                 "which one am I changing?", and it is also the one field that
                 cannot change, since link specs and the keychain entry are
                 both filed under it. -->
            <input id="cname" type="text" v-model="name" :disabled="!!props.editing"
                   placeholder="nas" spellcheck="false" />
            <span class="note">{{ props.editing
              ? "A name cannot change: your saved pairs refer to it, and so does the stored password."
              : "You will write this before a colon, like nas:inbox." }}</span>
          </div>

          <div class="opt">
            <label for="cscheme">Protocol</label>
            <select id="cscheme" v-model="scheme">
              <option value="ftp">FTP</option>
              <option value="ftps">FTP over TLS</option>
              <option value="fs">A folder this machine can already reach</option>
            </select>
            <span class="note">{{ local
              ? "A mounted volume or any local directory, driven the same way a remote is."
              : "Reached over the network, with a password." }}</span>
          </div>
        </div>

        <div v-if="clear" class="notice bad">
          <b>FTP sends your password and your files across the network unencrypted.</b>
          If the server offers it, choose FTP over TLS instead.
        </div>

        <template v-if="!local">
          <div class="pair">
            <div class="opt">
              <label for="chost">Server</label>
              <input id="chost" type="text" v-model="host" placeholder="nas.local"
                     spellcheck="false" />
            </div>
            <div class="opt">
              <label for="cport">Port</label>
              <input id="cport" type="text" v-model="port" placeholder="21" spellcheck="false" />
              <span class="note">Leave blank for the protocol default.</span>
            </div>
          </div>

          <div class="pair">
            <div class="opt">
              <label for="cuser">Sign in as</label>
              <input id="cuser" type="text" v-model="username" placeholder="me"
                     spellcheck="false" />
            </div>
            <div v-if="!props.editing" class="opt">
              <label for="cpass">Password</label>
              <input id="cpass" type="password" v-model="secret" autocomplete="off" />
              <span class="note">Kept in this machine's keychain, never in tungstate's own files.</span>
            </div>
          </div>
        </template>

        <div class="opt">
          <label for="croot">Folder to start from</label>
          <input id="croot" type="text" v-model="root"
                 :placeholder="local ? '/Volumes/nas' : '/volume1/media'" spellcheck="false" />
          <span class="note">{{ local
            ? "Everything you browse or drain is inside this directory."
            : "A path on the server, which is often not where you land when you sign in. Test will show you what is actually there." }}</span>
        </div>

        <div v-if="nameProblem" class="notice bad">{{ nameProblem }}</div>
        <div v-if="error" class="notice bad">{{ error }}</div>
      </div>

      <div class="feet">
        <span class="spacer"></span>
        <button class="btn" @click="emit('cancel')">Cancel</button>
        <button class="btn primary" :disabled="!ready || saving" @click="save">
          {{ saving ? "Saving…" : props.editing ? "Save changes" : "Add it" }}
        </button>
      </div>
    </div>
  </div>
</template>
