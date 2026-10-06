<!-- Add a connection, or edit one. Adding starts by choosing what kind of
     place it is, then asks only what that kind needs, each field explained.
     Check works before anything is saved. -->
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { connections, folders } from "../../engine/commands";
import type { Connection, ConnectionForm, Probe } from "../../engine/types";
import {
  KINDS, S3_SERVICES, SOON, WEBDAV_SERVICES, kindOf, serviceOf, splitShare, webdavServiceOf,
  type Kind, type S3Service, type WebDavService,
} from "../../lib/kinds";
import { plural } from "../../lib/format";
import Sheet from "../../ui/Sheet.vue";
import Field from "../../ui/Field.vue";
import Button from "../../ui/Button.vue";
import ActionBar from "../../ui/ActionBar.vue";
import Notice from "../../ui/Notice.vue";
import Segmented from "../../ui/Segmented.vue";

const props = defineProps<{ editing: Connection | null }>();
const emit = defineEmits<{ dismiss: []; saved: [name: string] }>();

const was = props.editing;
const kind = ref<Kind | null>(was ? kindOf(was.scheme).id : null);
const name = ref(was?.name ?? "");
const host = ref(was?.host ?? "");
const port = ref(was?.port ? String(was.port) : "");
const username = ref(was?.username ?? "");
// Only when adding. A stored password cannot be read back into a form, so on
// edit a blank field would have to mean "leave it"; changing it is its own
// button on the row instead.
const secret = ref("");
// fs, ftp and ftps: the folder to start from. S3: the folder in the bucket.
// WebDAV: the folder inside the server's address.
const root = ref(was?.root ?? "");
// SMB keeps its root as two fields: the share, then the folder inside it.
const share = ref(splitShare(was?.root ?? "").share);
const inside = ref(splitShare(was?.root ?? "").inside);
const encrypt = ref(was?.options.encryption === "required");
const service = ref<S3Service>(serviceOf(was?.options.endpoint));
// WebDAV: which service, which fills in the address when it has a fixed one.
const davService = ref<WebDavService>(webdavServiceOf(was?.options.endpoint));
const endpoint = ref(was?.options.endpoint ?? "");
const bucket = ref(was?.options.bucket ?? "");
const region = ref(was?.options.region ?? "");
// SFTP: a key file instead of a password, and the server's key once trusted.
const signIn = ref<"password" | "key">(was?.options.key ? "key" : "password");
const keyFile = ref(was?.options.key ?? "");
const hostKey = ref(was?.options.host_key ?? "");
const SIGN_INS = [
  { id: "password", label: "Password" },
  { id: "key", label: "Key file" },
] as const;

const problems = ref<string[]>([]);
const trouble = ref<string | null>(null);
const probe = ref<Probe | null>(null);
const checking = ref(false);
const saving = ref(false);

// The two-character rule from `parse_end`: one character before a colon is a
// Windows drive letter, so a one-character name could never be written in a
// location.
const nameOk = computed(() => /^[A-Za-z0-9_-]{2,}$/.test(name.value.trim()));
const info = computed(() => (kind.value ? kindOf(kind.value) : null));
const s3Service = computed(() => S3_SERVICES.find((s) => s.id === service.value)!);
const davInfo = computed(() => WEBDAV_SERVICES.find((s) => s.id === davService.value)!);

function form(): ConnectionForm {
  const k = kind.value!;
  const typed = Number(port.value);
  const options: Record<string, string> = { ...(was?.options ?? {}) };
  // Sent whole, so an edit never drops what the command line put there;
  // the keys this form owns are set or cleared from its fields.
  delete options.encryption;
  delete options.bucket;
  delete options.region;
  delete options.endpoint;
  delete options.key;
  delete options.host_key;
  if (k === "smb" && encrypt.value) options.encryption = "required";
  if (k === "sftp") {
    if (signIn.value === "key" && keyFile.value.trim()) options.key = keyFile.value.trim();
    if (hostKey.value) options.host_key = hostKey.value;
  }
  if (k === "s3") {
    options.bucket = bucket.value.trim();
    if (region.value.trim()) options.region = region.value.trim();
    if (service.value !== "aws" && endpoint.value.trim()) options.endpoint = endpoint.value.trim();
  }
  if (k === "webdav" && endpoint.value.trim()) options.endpoint = endpoint.value.trim();
  // WebDAV's server is its endpoint, a URL with its own port.
  const networked = k !== "fs" && k !== "s3" && k !== "webdav";
  return {
    name: name.value.trim(),
    scheme: k,
    host: networked ? host.value.trim() || null : null,
    port: networked && port.value.trim() && Number.isFinite(typed) ? typed : null,
    username: k === "fs" ? null : username.value.trim() || null,
    root: k === "smb" ? [share.value.trim(), inside.value.trim()].filter(Boolean).join("/") : root.value.trim(),
    options,
  };
}

// A changed field makes the last check's answer about something else.
watch([kind, host, port, username, secret, root, share, inside, encrypt, service, davService, endpoint, bucket, region, signIn, keyFile], () => {
  probe.value = null;
  trouble.value = null;
});
// A WebDAV service with one address fills it in; one with a pattern leaves
// the field to be filled, clearing an address another service put there.
watch(davService, () => {
  const fixed = davInfo.value.address;
  if (fixed) endpoint.value = fixed;
  else if (WEBDAV_SERVICES.some((s) => s.address && s.address === endpoint.value)) endpoint.value = "";
});
// Choosing a service fills in what it always needs, when nothing was typed.
watch(service, (now) => {
  if (!region.value || region.value === "auto" || region.value === "us-east-1") region.value = s3Service.value.region;
  if (now === "aws") endpoint.value = "";
});

/** The engine's own refusals, so this form and `connection add` refuse the
 *  same things in the same words. */
async function refused(): Promise<boolean> {
  problems.value = await connections.problems(form());
  return problems.value.length > 0;
}


async function check() {
  checking.value = true;
  trouble.value = null;
  probe.value = null;
  try {
    if (await refused()) return;
    probe.value = await connections.testSettings(form(), kind.value === "fs" ? null : secret.value);
  } catch (e) {
    trouble.value = String(e);
  } finally {
    checking.value = false;
  }
}

/** The person has agreed this is their server: keep its key with the form,
 *  saved with the connection, and check again, which now goes in. */
async function trust() {
  const server = probe.value?.server;
  if (!server) return;
  hostKey.value = server.key;
  await check();
}

async function save() {
  saving.value = true;
  trouble.value = null;
  try {
    if (await refused()) return;
    if (was) await connections.update(was.name, form());
    else await connections.add(form(), kind.value === "fs" ? null : secret.value);
    secret.value = ""; // held no longer than the call that needed it
    emit("saved", name.value.trim());
  } catch (e) {
    trouble.value = String(e);
  } finally {
    saving.value = false;
  }
}

async function chooseFolder() {
  const picked = await folders.pick();
  if (picked) root.value = picked;
}

/** What the check found, in one sentence. Listing and writing are different
 *  permissions, and a place that lists but refuses files is the case this
 *  sentence exists for. */
const verdict = computed(() => {
  const got = probe.value;
  if (!got || got.server) return null;
  const seen = `${plural(got.entries, "thing")} in ${got.root}${got.names.length ? `: ${got.names.slice(0, 6).join(", ")}` : ""}`;
  if (got.accepts_files === false) return { ok: false, line: `Reached it and found ${seen}, but it refuses to take a file. Choose a folder this account can write to.` };
  return { ok: true, line: `Reached it: ${seen}.${got.accepts_files ? " It accepts files." : ""}` };
});
</script>

<template>
  <Sheet wide of="connections" :title="was ? `Edit ${was.name}` : 'Add a connection'" @dismiss="emit('dismiss')">
    <template v-if="!kind">
      <p class="cf-why">What kind of place is it?</p>
      <div class="cf-kinds">
        <button v-for="k in KINDS" :key="k.id" class="cf-kind" @click="kind = k.id">
          <b><span class="cf-tag">{{ k.tag }}</span> {{ k.label }}</b>
          <span>{{ k.line }}</span>
        </button>
        <!-- Planned, said here so nobody wonders whether it ever will be. -->
        <div v-for="s in SOON" :key="s.label" class="cf-kind cf-soon" aria-disabled="true">
          <b><span class="cf-tag">Soon</span> {{ s.label }}</b>
          <span>{{ s.line }}</span>
        </div>
      </div>
    </template>

    <template v-else>
      <p class="cf-why">
        <span class="cf-tag">{{ info?.tag }}</span> {{ info?.label }}
        <Button v-if="!was" look="link" @click="kind = null">Change</Button>
      </p>

      <!-- On edit the name is said, not offered: saved pairs and the stored
           password are both filed under it, so it cannot change. -->
      <div class="cf-grid" v-if="!was">
        <Field label="Name" note="Short, like nas.">
          <input v-model="name" placeholder="nas" spellcheck="false" />
        </Field>
      </div>

      <template v-if="kind === 'fs'">
        <Field class="cf-gap" label="Folder" note="Everything browsed or sent is inside this folder. A mounted share is in /Volumes.">
          <div class="cf-inline">
            <input v-model="root" placeholder="/Volumes/nas" spellcheck="false" />
            <Button @click="chooseFolder()">Choose…</Button>
          </div>
        </Field>
      </template>

      <template v-if="kind === 'smb'">
        <div class="cf-grid">
          <Field label="Server" note="Its name on the network, or its address.">
            <input v-model="host" placeholder="nas.local" spellcheck="false" />
          </Field>
          <Field label="Share" note="The shared folder, as Finder lists it under the server.">
            <input v-model="share" placeholder="media" spellcheck="false" />
          </Field>
        </div>
        <Field class="cf-gap" label="Folder inside the share" note="Optional. Everything is kept inside it.">
          <input v-model="inside" placeholder="backups" spellcheck="false" />
        </Field>
      </template>

      <template v-if="kind === 'ftp' || kind === 'ftps'">
        <div class="cf-grid">
          <Field label="Server"><input v-model="host" placeholder="nas.local" spellcheck="false" /></Field>
          <Field label="Port" note="Leave blank for the usual one."><input v-model="port" placeholder="21" spellcheck="false" /></Field>
        </div>
        <Field class="cf-gap" label="Folder to start from" note="A path on the server. It is often not where you land when you sign in; Check shows what is really there.">
          <input v-model="root" placeholder="/volume1/media" spellcheck="false" />
        </Field>
      </template>

      <template v-if="kind === 'sftp'">
        <div class="cf-grid">
          <Field label="Server" note="Its name on the network, or its address.">
            <input v-model="host" placeholder="nas.local" spellcheck="false" />
          </Field>
          <Field label="Port" note="Leave blank for the usual one."><input v-model="port" placeholder="22" spellcheck="false" /></Field>
        </div>
        <Field class="cf-gap" label="Folder to start from" note="Optional. A path on the server, like /volume1/media. Leave it blank to start where the account signs in.">
          <input v-model="root" placeholder="/volume1/media" spellcheck="false" />
        </Field>
        <div class="cf-signin">
          <span class="cf-signin-label">Sign in with</span>
          <Segmented v-model="signIn" :options="SIGN_INS" label="Sign in with" />
        </div>
        <Field v-if="signIn === 'key'" class="cf-gap" label="Key file" note="A private key on this Mac, like the one ssh uses.">
          <input v-model="keyFile" placeholder="~/.ssh/id_ed25519" spellcheck="false" />
        </Field>
      </template>

      <template v-if="kind === 'webdav'">
        <Field class="cf-gap" label="Service" :note="davInfo.note || undefined">
          <select v-model="davService">
            <option v-for="s in WEBDAV_SERVICES" :key="s.id" :value="s.id">{{ s.label }}</option>
          </select>
        </Field>
        <Field v-if="!davInfo.address" class="cf-gap" label="Address" note="The server's web address, with its port if it has one.">
          <input v-model="endpoint" :placeholder="davInfo.template" spellcheck="false" />
        </Field>
        <Field class="cf-gap" label="Folder to start from" note="Optional. A folder at that address; everything is kept inside it.">
          <input v-model="root" placeholder="media" spellcheck="false" />
        </Field>
      </template>

      <template v-if="kind === 's3'">
        <div class="cf-grid">
          <Field label="Service" :note="s3Service.keys">
            <select v-model="service">
              <option v-for="s in S3_SERVICES" :key="s.id" :value="s.id">{{ s.label }}</option>
            </select>
          </Field>
          <Field label="Bucket"><input v-model="bucket" placeholder="family-photos" spellcheck="false" /></Field>
        </div>
        <div class="cf-grid">
          <Field v-if="service !== 'aws'" label="Endpoint" note="The service's address, from its dashboard.">
            <input v-model="endpoint" :placeholder="s3Service.example" spellcheck="false" />
          </Field>
          <Field label="Region" :note="service === 'aws' ? 'Where the bucket is, like eu-west-2.' : 'Leave as it is unless the service says otherwise.'">
            <input v-model="region" :placeholder="s3Service.region || 'us-east-1'" spellcheck="false" />
          </Field>
        </div>
        <Field class="cf-gap" label="Folder inside the bucket" note="Optional. Everything is kept inside it.">
          <input v-model="root" placeholder="2026" spellcheck="false" />
        </Field>
      </template>

      <div class="cf-grid" v-if="kind !== 'fs'">
        <Field :label="kind === 's3' ? 'Access key ID' : 'Sign in as'">
          <input v-model="username" :placeholder="kind === 's3' ? 'AKIA…' : 'me'" spellcheck="false" />
        </Field>
        <Field
          v-if="!was"
          :label="kind === 's3' ? 'Secret key' : kind === 'sftp' && signIn === 'key' ? 'Passphrase' : 'Password'"
          :note="kind === 'sftp' && signIn === 'key' ? 'Only if the key has one. Kept in this machine\'s keychain.' : 'Kept in this machine\'s keychain, never in Tungstate\'s own files.'"
        >
          <input type="password" v-model="secret" autocomplete="off" />
        </Field>
      </div>

      <label class="cf-tick" v-if="kind === 'smb'">
        <input type="checkbox" v-model="encrypt" />
        <span>
          Only connect if the server encrypts
          <em>Without this, files cross the network encrypted only if the server asks. Your password is never sent as it is either way.</em>
        </span>
      </label>
      <Notice tone="hold" v-if="kind === 'ftp'">
        FTP sends your password and your files across the network unencrypted. If the server offers
        it, choose FTP over TLS.
      </Notice>
      <Notice tone="hold" v-if="kind === 'webdav' && endpoint.trim().toLowerCase().startsWith('http://')">
        This address is plain http, so your password and your files cross the network unencrypted. Use https if the server offers it.
      </Notice>
      <Notice tone="hold" v-if="kind === 's3' && endpoint.trim().toLowerCase().startsWith('http://')">
        This endpoint is plain http, so your files cross the network unencrypted. Your secret key never does.
      </Notice>

      <ActionBar pinned>
        <!-- What is wrong with the form, and what a check said, sit on the bar
             itself, so neither can be scrolled out of sight behind it. -->
        <template #above>
          <Notice tone="bad" v-if="name.trim() && !nameOk">Use two or more letters, digits, dashes or underscores for the name.</Notice>
          <Notice tone="bad" v-if="problems.length">
            <template v-for="(p, i) in problems" :key="i">{{ p.charAt(0).toUpperCase() + p.slice(1) }}.<br v-if="i < problems.length - 1" /></template>
          </Notice>
          <Notice v-if="probe?.server && !probe.server.changed" tone="hold">
            This is the first time Tungstate has reached {{ host.trim() || "this server" }}. Its fingerprint is
            <code class="cf-print">{{ probe.server.fingerprint }}</code>. If it matches what the server shows, trust it.
            <template #act><Button :busy="checking" @click="trust()">Trust it</Button></template>
          </Notice>
          <Notice v-if="probe?.server?.changed" tone="bad">
            This is not the server trusted before: its key has changed, and its fingerprint is now
            <code class="cf-print">{{ probe.server.fingerprint }}</code>. That happens when a server is reset, and when
            something pretends to be it. Only trust it if you know which.
            <template #act><Button look="danger" :busy="checking" @click="trust()">Trust the new key</Button></template>
          </Notice>
          <Notice v-if="verdict" :tone="verdict.ok ? 'plain' : 'bad'">{{ verdict.line }}</Notice>
          <Notice tone="bad" v-if="trouble">{{ trouble }}</Notice>
        </template>
        <Button @click="emit('dismiss')">Cancel</Button>
        <Button :disabled="!nameOk" :busy="checking" @click="check()">Check</Button>
        <Button look="primary" :disabled="!nameOk" :busy="saving" @click="save()">
          {{ was ? "Save changes" : "Add it" }}
        </Button>
      </ActionBar>
    </template>
  </Sheet>
</template>

<style scoped>
.cf-why { font-size: var(--small); color: var(--text-quiet); margin: var(--s2) 0 var(--s4); line-height: 1.5; }
.cf-kinds { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--s2); }
.cf-kind {
  font: inherit;
  text-align: left;
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: var(--s3);
  background: var(--surface-raised);
  color: var(--text);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius);
  cursor: pointer;
}
.cf-kind:hover { background: var(--surface-hover); }
.cf-kind b { font-size: var(--small); }
.cf-kind span { font-size: var(--fine); color: var(--text-quiet); line-height: 1.45; }
.cf-grid { display: grid; grid-template-columns: 1fr 1fr; gap: var(--s3); margin-bottom: var(--s3); }
.cf-gap { margin-bottom: var(--s3); }
.cf-inline { display: flex; gap: var(--s2); }
.cf-tick { display: flex; gap: var(--s2); align-items: flex-start; font-size: var(--small); margin: var(--s2) 0 var(--s3); }
.cf-tick em { display: block; font-style: normal; font-size: var(--fine); color: var(--text-faint); margin-top: 2px; line-height: 1.45; }
.cf-signin { display: flex; align-items: center; gap: var(--s3); margin-bottom: var(--s3); }
.cf-signin-label { font-size: var(--small); font-weight: 600; }
.cf-print { font-family: var(--font-mono); font-size: var(--fine); word-break: break-all; }
.cf-soon { cursor: default; opacity: 0.7; }
.cf-soon:hover { background: var(--surface-raised); }
.cf-tag {
  font-size: var(--fine);
  font-weight: 600;
  color: var(--text-quiet);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius);
  padding: 0 5px;
  margin-right: 2px;
}
</style>
