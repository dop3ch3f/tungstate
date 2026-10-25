<!-- Add a connection: choose its kind, then answer only what that kind needs.
     Check works before anything is saved; Save keeps the password in this
     phone's keychain. The settings are built the way the desktop's form
     builds them, so a connection means the same thing on both.

     Not on the phone yet: SFTP sign-in with a key file, since a phone has
     no file path to point at. -->
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { connections } from "../../engine/commands";
import type { ConnectionForm, Probe } from "../../engine/types";
import { KINDS, type Kind } from "../../lib/kinds";
import { plural } from "../../lib/format";
import PScreen from "../ui/PScreen.vue";
import PRow from "../ui/PRow.vue";
import PTile from "../ui/PTile.vue";
import PButton from "../ui/PButton.vue";

const emit = defineEmits<{ cancel: []; saved: [name: string] }>();

// A folder the device mounts means nothing on a phone, and a key file needs
// a path the phone does not have, so SFTP signs in by password here.
const CHOICES = KINDS.filter((k) => k.id !== "fs").map((k) =>
  k.id === "sftp" ? { ...k, line: "Encrypted, and on almost every NAS. Signs in with a password." } : k,
);

const kind = ref<Kind | null>(null);
const info = computed(() => CHOICES.find((k) => k.id === kind.value) ?? null);

const name = ref("");
const host = ref("");
const port = ref("");
const share = ref("");
const root = ref("");
const endpoint = ref("");
const bucket = ref("");
const region = ref("");
const username = ref("");
const secret = ref("");
/** An SFTP server's key, once the person has agreed it is theirs. */
const hostKey = ref("");

const checking = ref(false);
const saving = ref(false);
const probe = ref<Probe | null>(null);
const trouble = ref<string | null>(null);

// Read on every input event rather than through v-model. Android keyboards
// "compose" a word until it is finished, and v-model ignores input while a
// composition is open, so a typed address looked filled and read as empty.
const typed = (e: Event) => (e.target as HTMLInputElement).value;

const DEFAULT_NAME: Record<string, string> = { smb: "NAS", sftp: "NAS", webdav: "WebDAV", s3: "Bucket", ftp: "FTP", ftps: "FTP" };
function choose(k: Kind) {
  kind.value = k;
  name.value = DEFAULT_NAME[k] ?? "";
}

function form(): ConnectionForm {
  const k = kind.value!;
  const options: Record<string, string> = {};
  if (k === "sftp" && hostKey.value) options.host_key = hostKey.value;
  if (k === "s3") {
    options.bucket = bucket.value.trim();
    if (region.value.trim()) options.region = region.value.trim();
    if (endpoint.value.trim()) options.endpoint = endpoint.value.trim();
  }
  if (k === "webdav" && endpoint.value.trim()) options.endpoint = endpoint.value.trim();
  // WebDAV's server is its address, and S3's is its endpoint or its region.
  const hosted = k !== "s3" && k !== "webdav";
  const n = Number(port.value);
  return {
    name: name.value.trim(),
    scheme: k,
    host: hosted ? host.value.trim() || null : null,
    port: hosted && port.value.trim() && Number.isFinite(n) ? n : null,
    username: username.value.trim() || null,
    root: k === "smb" ? [share.value.trim(), root.value.trim()].filter(Boolean).join("/") : root.value.trim(),
    options,
  };
}

/** What a kind cannot be saved without; the engine's own checks follow. */
const ready = computed(() => {
  const k = kind.value;
  if (!k || !/^[A-Za-z0-9_-]{2,}$/.test(name.value.trim()) || !secret.value) return false;
  if (k === "s3") return !!bucket.value.trim() && !!username.value.trim();
  if (k === "webdav") return !!endpoint.value.trim();
  if (k === "smb") return !!host.value.trim() && !!share.value.trim() && !!username.value.trim();
  return !!host.value.trim() && !!username.value.trim();
});

// A changed field makes the last check's answer about something else.
watch([kind, name, host, port, share, root, endpoint, bucket, region, username, secret, hostKey], () => {
  probe.value = null;
  trouble.value = null;
});

async function refused(): Promise<boolean> {
  const problems = await connections.problems(form());
  trouble.value = problems.length ? problems.join(" ") : null;
  return problems.length > 0;
}

async function check() {
  checking.value = true;
  try {
    if (await refused()) return;
    probe.value = await connections.testSettings(form(), secret.value);
  } catch (e) {
    trouble.value = String(e);
  } finally {
    checking.value = false;
  }
}

/** The person has compared the fingerprint with their server's: keep it and check again. */
async function trust() {
  hostKey.value = probe.value?.server?.key ?? "";
  await check();
}

async function save() {
  saving.value = true;
  try {
    if (await refused()) return;
    await connections.add(form(), secret.value);
    emit("saved", form().name);
  } catch (e) {
    trouble.value = String(e);
  } finally {
    saving.value = false;
  }
}

const NAME_HINT = "Letters, numbers, - and _; at least two.";
</script>

<template>
  <PScreen v-if="!info" title="Add a connection" back="Connections" @back="emit('cancel')">
    <div class="list">
      <PRow v-for="k in CHOICES" :key="k.id" :title="k.label" :sub="k.line" chevron @press="choose(k.id)">
        <template #lead><span class="tag">{{ k.tag }}</span></template>
      </PRow>
    </div>
  </PScreen>

  <PScreen v-else :title="info.label" back="Kinds" @back="kind = null">
    <div class="fields">
      <label class="field"><span>Name <em>{{ NAME_HINT }}</em></span><input :value="name" autocapitalize="off" autocorrect="off" spellcheck="false" @input="name = typed($event)" /></label>

      <template v-if="kind === 'webdav'">
        <label class="field"><span>Server address</span><input :value="endpoint" placeholder="https://dav.example.com/remote.php/dav/files/me" autocapitalize="off" autocorrect="off" spellcheck="false" inputmode="url" @input="endpoint = typed($event)" /></label>
      </template>
      <template v-else-if="kind === 's3'">
        <label class="field"><span>Endpoint <em>blank for AWS</em></span><input :value="endpoint" placeholder="https://s3.eu-central-003.backblazeb2.com" autocapitalize="off" autocorrect="off" spellcheck="false" inputmode="url" @input="endpoint = typed($event)" /></label>
        <label class="field"><span>Bucket</span><input :value="bucket" autocapitalize="off" autocorrect="off" spellcheck="false" @input="bucket = typed($event)" /></label>
        <label class="field"><span>Region <em>optional</em></span><input :value="region" placeholder="us-east-1" autocapitalize="off" autocorrect="off" spellcheck="false" @input="region = typed($event)" /></label>
      </template>
      <template v-else>
        <label class="field"><span>Address</span><input :value="host" placeholder="nas.local or 192.168.1.20" autocapitalize="off" autocorrect="off" spellcheck="false" inputmode="url" @input="host = typed($event)" /></label>
        <label class="field"><span>Port <em>optional</em></span><input :value="port" inputmode="numeric" @input="port = typed($event)" /></label>
        <label v-if="kind === 'smb'" class="field"><span>Share</span><input :value="share" placeholder="Media" autocapitalize="off" autocorrect="off" spellcheck="false" @input="share = typed($event)" /></label>
      </template>

      <label class="field"><span>{{ kind === 'smb' ? 'Folder inside it' : 'Folder' }} <em>optional</em></span><input :value="root" autocapitalize="off" autocorrect="off" spellcheck="false" @input="root = typed($event)" /></label>
      <label class="field"><span>{{ kind === 's3' ? 'Access key ID' : 'Username' }}</span><input :value="username" autocapitalize="off" autocorrect="off" spellcheck="false" autocomplete="username" @input="username = typed($event)" /></label>
      <label class="field"><span>{{ kind === 's3' ? 'Secret key' : 'Password' }}</span><input :value="secret" type="password" autocomplete="current-password" @input="secret = typed($event)" /></label>
    </div>

    <template #act>
    <div v-if="probe?.server" class="trust">
      <b v-if="probe.server.changed">This isn't the server you trusted before</b>
      <b v-else>Is this your server?</b>
      <p>It identifies itself as <code>{{ probe.server.fingerprint }}</code>. Compare it with what your server shows before trusting it.</p>
      <PButton quiet @press="trust">Trust this server</PButton>
    </div>
    <p v-else-if="trouble" class="line bad">{{ trouble }}</p>
    <p v-else-if="probe" class="line good">
      Reached it. {{ plural(probe.entries, "item") }} there{{ probe.accepts_files === false ? ", but it won't accept new files with these details." : "." }}
    </p>
    <p v-else class="line">Your password stays in this phone's keychain.</p>
      <div class="pair">
        <PButton quiet :busy="checking" :disabled="!ready" @press="check">{{ checking ? "Checking…" : "Check" }}</PButton>
        <PButton tint="var(--tint-connections)" :busy="saving" :disabled="!ready" @press="save">{{ saving ? "Saving…" : "Save" }}</PButton>
      </div>
    </template>
    <p class="about">{{ info.line }}</p>
  </PScreen>
</template>

<style scoped>
.list { background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; overflow: hidden; }
.tag { flex: none; min-width: 58px; padding: 6px 4px; text-align: center; font-size: 12px; font-weight: 800; color: #ffffff; background: var(--tint-connections); border: 2px solid var(--edge); border-radius: 6px; }
.fields { display: flex; flex-direction: column; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; overflow: hidden; }
.field { display: flex; flex-direction: column; gap: 4px; padding: 10px 14px; border-top: 2px solid var(--edge); }
.field:first-child { border-top: 0; }
.field span { font-size: 13px; font-weight: 800; }
.field em { font-style: normal; font-weight: 600; color: var(--text-quiet); }
.field input {
  min-height: 36px;
  padding: 0;
  font: inherit;
  /* 16px or more, or iOS zooms the page on focus. */
  font-size: 17px;
  color: var(--text);
  background: none;
  border: 0;
  outline: none;
  -webkit-user-select: text;
  user-select: text;
}
/* The answer to Check sits over the buttons, where the eye already is. */
.line { margin: 0 2px 10px; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.about { margin: 14px 2px 0; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.good { color: var(--ok); font-weight: 700; }
.bad { color: var(--bad); font-weight: 700; }
.trust { display: flex; flex-direction: column; gap: 10px; margin-bottom: 12px; padding: 14px 16px; background: var(--warn-bar); border: 2px solid var(--edge); border-radius: 8px; }
.trust b { font-size: 16px; }
.trust p { margin: 0; font-size: 15px; line-height: 1.4; }
.trust code { font-size: 13px; overflow-wrap: anywhere; }
.pair { display: flex; gap: 12px; }
.pair > * { flex: 1; }
</style>
