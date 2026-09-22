<!-- Add a connection, or edit one. A place files can go that is not this
     machine: once it exists it appears in each pane's "Go to…" list, mounted
     in Finder or not. -->
<script setup lang="ts">
import { computed, ref } from "vue";
import { connections } from "../../engine/commands";
import type { Connection, ConnectionForm } from "../../engine/types";
import Sheet from "../../ui/Sheet.vue";
import Field from "../../ui/Field.vue";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";

const props = defineProps<{ editing: Connection | null }>();
const emit = defineEmits<{ dismiss: []; saved: [name: string] }>();

const name = ref(props.editing?.name ?? "");
const scheme = ref(props.editing?.scheme ?? "ftp");
const host = ref(props.editing?.host ?? "");
const port = ref(props.editing?.port ? String(props.editing.port) : "");
const username = ref(props.editing?.username ?? "");
const root = ref(props.editing?.root ?? "");
// Only when adding. A stored password cannot be read back into a form, so on
// edit a blank field would have to mean "leave it"; changing it is its own
// button on the row instead.
const secret = ref("");
const saving = ref(false);
const problem = ref<string | null>(null);

const local = computed(() => scheme.value === "fs");
// The two-character rule from `parse_end`: one character before a colon is a
// Windows drive letter, so a one-character name could never be written in a
// location.
const nameOk = computed(() => /^[A-Za-z0-9_-]{2,}$/.test(name.value.trim()));
const ready = computed(() => nameOk.value && (local.value || !!host.value.trim()));

function form(): ConnectionForm {
  const typed = Number(port.value);
  return {
    name: name.value.trim(),
    scheme: scheme.value,
    host: local.value ? null : host.value.trim() || null,
    port: !local.value && port.value.trim() && Number.isFinite(typed) ? typed : null,
    username: local.value ? null : username.value.trim() || null,
    root: root.value.trim(),
    // No per-scheme extras on screen. Sent back so an edit never drops what
    // the command line put there.
    options: props.editing?.options ?? {},
  };
}

async function save() {
  saving.value = true;
  problem.value = null;
  try {
    if (props.editing) await connections.update(props.editing.name, form());
    else await connections.add(form(), local.value ? null : secret.value);
    secret.value = ""; // held no longer than the call that needed it
    emit("saved", name.value.trim());
  } catch (e) {
    problem.value = String(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <Sheet wide @dismiss="emit('dismiss')">
    <h2 class="cf-title">{{ props.editing ? `Edit ${props.editing.name}` : "Add a connection" }}</h2>
    <p class="cf-why">
      A place files can go that is not this machine. Once it is here you can browse it in a pane
      and move files onto it, whether or not it is mounted in Finder.
    </p>

    <div class="cf-grid">
      <!-- Disabled rather than hidden on edit: it answers "which one am I
           changing?", and it cannot change, because saved pairs and the
           stored password are both filed under it. -->
      <Field label="Name" :note="props.editing ? 'Saved pairs and the stored password refer to this name, so it cannot change.' : 'You type this before a colon, like nas:inbox.'">
        <input v-model="name" :disabled="!!props.editing" placeholder="nas" spellcheck="false" />
      </Field>
      <Field label="Protocol">
        <select v-model="scheme">
          <option value="ftp">FTP</option>
          <option value="ftps">FTP over TLS</option>
          <option value="fs">A folder this machine can already reach</option>
        </select>
      </Field>
    </div>

    <Notice tone="hold" v-if="scheme === 'ftp'">
      FTP sends your password and your files across the network unencrypted. If the server
      offers it, choose FTP over TLS.
    </Notice>

    <template v-if="!local">
      <div class="cf-grid">
        <Field label="Server"><input v-model="host" placeholder="nas.local" spellcheck="false" /></Field>
        <Field label="Port" note="Leave blank for the usual one."><input v-model="port" placeholder="21" spellcheck="false" /></Field>
      </div>
      <div class="cf-grid">
        <Field label="Sign in as"><input v-model="username" placeholder="me" spellcheck="false" /></Field>
        <Field v-if="!props.editing" label="Password" note="Kept in this machine's keychain, never in Tungstate's own files.">
          <input type="password" v-model="secret" autocomplete="off" />
        </Field>
      </div>
    </template>

    <Field
      label="Folder to start from"
      :note="local ? 'Everything you browse or send is inside this folder.' : 'A path on the server, which is often not where you land when you sign in. Check shows what is really there.'"
    >
      <input v-model="root" :placeholder="local ? '/Volumes/nas' : '/volume1/media'" spellcheck="false" />
    </Field>

    <Notice tone="bad" v-if="name.trim() && !nameOk">
      Use two or more letters, digits, dashes or underscores.
    </Notice>
    <Notice tone="bad" v-if="problem">{{ problem }}</Notice>

    <div class="cf-foot">
      <Button @click="emit('dismiss')">Cancel</Button>
      <Button look="primary" :disabled="!ready || saving" @click="save()">
        {{ saving ? "Saving…" : props.editing ? "Save changes" : "Add it" }}
      </Button>
    </div>
  </Sheet>
</template>

<style scoped>
.cf-title { font-size: var(--body); font-weight: 700; margin: 0; }
.cf-why { font-size: var(--small); color: var(--text-quiet); margin: var(--s2) 0 var(--s4); line-height: 1.5; }
.cf-grid { display: grid; grid-template-columns: 1fr 1fr; gap: var(--s3); margin-bottom: var(--s3); }
.cf-foot { display: flex; justify-content: flex-end; gap: var(--s2); margin-top: var(--s5); }
</style>
