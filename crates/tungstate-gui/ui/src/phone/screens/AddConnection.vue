<!-- Add a connection. From a server found on the network, the kind, address
     and name are filled in and only the username and password are typed; by
     hand, a kind is chosen first. An SMB server lists its shares once signed
     in to, so a share is picked rather than typed. FTP and SMB can sign in as
     a guest where the server allows it. Nothing is saved until it has worked.
     The settings are built the way the desktop's form builds them, so a
     connection means the same thing on both.

     Not on the phone yet: SFTP sign-in with a key file, since a phone has
     no file path to point at. -->
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { connections, network } from "../../engine/commands";
import type { ConnectionForm, FoundServer, Probe } from "../../engine/types";
import { KINDS, type Kind } from "../../lib/kinds";
import { plural } from "../../lib/format";
import { useConnections } from "../../state/useConnections";
import { kindsOf, nameFor, portFor, type FoundKind } from "../lib/discovery";
import { useBack } from "../lib/back";
import PScreen from "../ui/PScreen.vue";
import PRow from "../ui/PRow.vue";
import PButton from "../ui/PButton.vue";

const props = defineProps<{ found?: FoundServer | null }>();
const emit = defineEmits<{ cancel: []; saved: [name: string] }>();
const conns = useConnections();

// A folder the device mounts means nothing on a phone, and a key file needs
// a path the phone does not have, so SFTP signs in by password here.
const CHOICES = KINDS.filter((k) => k.id !== "fs").map((k) =>
  k.id === "sftp" ? { ...k, line: "Encrypted, and on almost every NAS. Signs in with a password." } : k,
);
const TAG: Record<FoundKind, string> = { smb: "SMB", sftp: "SFTP", webdav: "WebDAV", webdavs: "WebDAV", ftp: "FTP" };

const kind = ref<Kind | null>(null);
const foundKind = ref<FoundKind | null>(null);
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
const guest = ref(false);
const showSecret = ref(false);
/** An SFTP server's key, once the person has agreed it is theirs. */
const hostKey = ref("");
/** The shares the SMB server listed once signed in to; null until then. */
const shares = ref<string[] | null>(null);
/** The server would not list its shares, so the share is typed instead. */
const typeShare = ref(false);

const busy = ref(false);
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

/** Fills the form from a found server, for one of the kinds it offers. */
function fromFound(f: FoundServer, k: FoundKind) {
  foundKind.value = k;
  kind.value = k === "webdavs" ? "webdav" : k;
  name.value = nameFor(f, conns.all.value.map((c) => c.name));
  const p = portFor(f, k);
  host.value = f.address;
  port.value = p === null ? "" : String(p);
  if (k === "webdav" || k === "webdavs") {
    const scheme = k === "webdavs" ? "https" : "http";
    endpoint.value = `${scheme}://${f.address}${p === null ? "" : `:${p}`}/`;
  }
  shares.value = null;
  typeShare.value = false;
}
watch(() => props.found, (f) => f && fromFound(f, kindsOf(f)[0]!), { immediate: true });
const offered = computed(() => (props.found ? [...new Map(kindsOf(props.found).map((k) => [TAG[k], k])).values()] : []));

const canGuest = computed(() => kind.value === "smb" || kind.value === "ftp");
/** What a guest signs in as: what servers that allow guests expect. */
const user = computed(() => (guest.value && canGuest.value ? (kind.value === "ftp" ? "anonymous" : "guest") : username.value.trim()));
const pass = computed(() => (guest.value && canGuest.value ? (kind.value === "ftp" ? "anonymous@" : "") : secret.value));

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
    username: user.value || null,
    root: k === "smb" ? [share.value.trim(), root.value.trim()].filter(Boolean).join("/") : root.value.trim(),
    options,
  };
}

const nameOk = computed(() => /^[A-Za-z0-9_-]{2,}$/.test(name.value.trim()));
const signedIn = computed(() => (guest.value && canGuest.value) || (!!user.value && !!pass.value));
/** Enough to try: what each kind cannot work without. */
const ready = computed(() => {
  const k = kind.value;
  if (!k || !nameOk.value) return false;
  if (k === "s3") return !!bucket.value.trim() && !!username.value.trim() && !!secret.value;
  if (k === "webdav") return !!endpoint.value.trim() && signedIn.value;
  return !!host.value.trim() && signedIn.value;
});
/** SMB needs its share before it can be saved; the rest are ready as they are. */
const needsShare = computed(() => kind.value === "smb" && !share.value.trim());

// A changed field makes the last check's answer about something else.
watch([kind, host, port, username, secret, guest, endpoint, bucket, region], () => {
  probe.value = null;
  trouble.value = null;
  shares.value = null;
});
watch([share, root, hostKey, name], () => {
  probe.value = null;
  trouble.value = null;
});

async function refused(): Promise<boolean> {
  const problems = await connections.problems(form());
  trouble.value = problems.length ? problems.join(" ") : null;
  return problems.length > 0;
}

/** Signs in to an SMB server and lists its shares to pick from. */
async function listShares() {
  busy.value = true;
  trouble.value = null;
  try {
    const n = Number(port.value);
    const got = await network.shares(host.value.trim(), port.value.trim() && Number.isFinite(n) ? n : null, user.value, pass.value);
    shares.value = got;
    if (got.length === 1) share.value = got[0]!;
    if (!got.length) {
      typeShare.value = true;
      trouble.value = "Signed in, but the server lists no shares. Type the share's name.";
    }
  } catch (e) {
    const said = String(e);
    trouble.value = said;
    // Signed in but the listing is refused: some servers allow the one and
    // not the other. Typing the name still works.
    if (!/password|sign in|auth/i.test(said)) typeShare.value = true;
  } finally {
    busy.value = false;
  }
}

/** Checks the settings work, then saves them. One press, as ES does. */
async function signIn() {
  busy.value = true;
  try {
    if (await refused()) return;
    probe.value = await connections.testSettings(form(), pass.value || null);
    // An SFTP server nobody has trusted yet: ask before going on.
    if (probe.value.server) return;
    await connections.add(form(), pass.value || null);
    emit("saved", form().name);
  } catch (e) {
    trouble.value = String(e);
  } finally {
    busy.value = false;
  }
}

/** The person has compared the fingerprint with their server's: keep it and go on. */
async function trust() {
  hostKey.value = probe.value?.server?.key ?? "";
  await signIn();
}

/** The one main button: what pressing it does next. */
const step = computed<{ label: string; run: () => void; ok: boolean }>(() => {
  if (kind.value === "smb" && shares.value === null && !typeShare.value) {
    return { label: busy.value ? "Signing in…" : "Sign in", run: listShares, ok: ready.value };
  }
  const where = kind.value === "smb" && share.value.trim() ? ` and open ${share.value.trim()}` : "";
  return { label: busy.value ? "Checking…" : `Save${where}`, run: signIn, ok: ready.value && !needsShare.value };
});

const back = () => (props.found || !kind.value ? emit("cancel") : (kind.value = null));
// Android's back steps out of the form, as its own back link does.
useBack(() => (back(), true));
const title = computed(() => (props.found ? `Add ${props.found.name ?? props.found.address}` : info.value?.label ?? "Add a connection"));
const NAME_HINT = "Letters, numbers, - and _; at least two.";
const ftpWarning = computed(() => {
  if (kind.value !== "ftp") return null;
  const better = offered.value.find((k) => k === "smb" || k === "sftp");
  return better ? `FTP sends your password unencrypted. ${props.found?.name ?? "This server"} also offers ${TAG[better]}, which doesn't.` : "FTP sends your password unencrypted.";
});
</script>

<template>
  <PScreen v-if="!info" title="Add a connection" back="Connections" @back="emit('cancel')">
    <div class="list">
      <PRow v-for="k in CHOICES" :key="k.id" :title="k.label" :sub="k.line" chevron @press="choose(k.id)">
        <template #lead><span class="tag">{{ k.tag }}</span></template>
      </PRow>
    </div>
  </PScreen>

  <PScreen v-else :title="title" :back="found ? 'Connections' : 'Kinds'" @back="back">
    <p v-if="found" class="line">Found on your network.</p>
    <div v-if="offered.length > 1" class="seg" role="group" aria-label="Kind">
      <button v-for="k in offered" :key="k" :class="{ on: foundKind === k }" @click="fromFound(found!, k)">{{ TAG[k] }}</button>
    </div>

    <!-- The share step: signed in, now pick where to go. -->
    <template v-if="kind === 'smb' && shares?.length">
      <div class="list">
        <button v-for="s in shares" :key="s" class="pick" :class="{ on: share === s }" @click="share = s">
          <span class="radio" :class="{ on: share === s }" aria-hidden="true"></span>
          <b>{{ s }}</b>
        </button>
      </div>
      <div class="fields gap">
        <label class="field"><span>Folder inside it <em>optional</em></span><input :value="root" autocapitalize="off" autocorrect="off" spellcheck="false" @input="root = typed($event)" /></label>
        <label class="field"><span>Name in Tungstate <em>{{ NAME_HINT }}</em></span><input :value="name" autocapitalize="off" autocorrect="off" spellcheck="false" @input="name = typed($event)" /></label>
      </div>
    </template>

    <div v-else class="fields">
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
        <label v-if="!found || port" class="field"><span>Port <em>optional</em></span><input :value="port" inputmode="numeric" @input="port = typed($event)" /></label>
      </template>

      <button v-if="canGuest" class="field row2" role="switch" :aria-checked="guest" @click="guest = !guest">
        <span class="grow"><b>Sign in as guest</b><em>If the server allows it</em></span>
        <span class="switch" :class="{ on: guest }" aria-hidden="true"></span>
      </button>
      <template v-if="!guest || !canGuest">
        <label class="field"><span>{{ kind === 's3' ? 'Access key ID' : 'Username' }}</span><input :value="username" autocapitalize="off" autocorrect="off" spellcheck="false" autocomplete="username" @input="username = typed($event)" /></label>
        <label class="field">
          <span>{{ kind === 's3' ? 'Secret key' : 'Password' }}</span>
          <span class="secret">
            <input :value="secret" :type="showSecret ? 'text' : 'password'" autocomplete="current-password" autocapitalize="off" autocorrect="off" spellcheck="false" @input="secret = typed($event)" />
            <button type="button" class="show" @click.prevent="showSecret = !showSecret">{{ showSecret ? "Hide" : "Show" }}</button>
          </span>
        </label>
      </template>

      <label v-if="kind === 'smb' && typeShare" class="field"><span>Share</span><input :value="share" placeholder="Media" autocapitalize="off" autocorrect="off" spellcheck="false" @input="share = typed($event)" /></label>
      <label v-if="kind !== 'smb'" class="field"><span>Folder <em>optional</em></span><input :value="root" autocapitalize="off" autocorrect="off" spellcheck="false" @input="root = typed($event)" /></label>
      <label v-if="!found || kind !== 'smb'" class="field"><span>Name in Tungstate <em>{{ NAME_HINT }}</em></span><input :value="name" autocapitalize="off" autocorrect="off" spellcheck="false" @input="name = typed($event)" /></label>
    </div>

    <p v-if="ftpWarning" class="note">{{ ftpWarning }}</p>
    <p v-else-if="!found" class="about">{{ info.line }}</p>

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
      <p v-else-if="kind === 'smb' && shares === null && !typeShare" class="line">Tungstate lists the server's shares once you're signed in.</p>
      <p v-else class="line">Your password stays in this phone's keystore.</p>
      <PButton v-if="!probe?.server" tint="var(--tint-connections)" :busy="busy" :disabled="!step.ok" @press="step.run">{{ step.label }}</PButton>
    </template>
  </PScreen>
</template>

<style scoped>
.list { background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; overflow: hidden; }
.tag { flex: none; min-width: 58px; padding: 6px 4px; text-align: center; font-size: 12px; font-weight: 800; color: #ffffff; background: var(--tint-connections); border: 2px solid var(--edge); border-radius: 6px; }
.seg { display: flex; margin-bottom: 14px; overflow: hidden; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.seg button { flex: 1; min-height: 48px; font: inherit; font-size: 15px; font-weight: 700; color: var(--text); background: none; border: 0; border-right: 2px solid var(--edge); }
.seg button:last-child { border-right: 0; }
.seg button.on { color: var(--panel); background: var(--edge); }
.fields { display: flex; flex-direction: column; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; overflow: hidden; }
.fields.gap { margin-top: 14px; }
.field { display: flex; flex-direction: column; gap: 4px; padding: 10px 14px; border-top: 2px solid var(--edge); }
.field:first-child { border-top: 0; }
.field > span { font-size: 13px; font-weight: 800; }
.field em { font-style: normal; font-weight: 600; color: var(--text-quiet); }
.field input {
  min-height: 36px;
  min-width: 0;
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
.secret { display: flex; align-items: center; gap: 10px; }
.secret input { flex: 1; }
.show { flex: none; height: 32px; padding: 0 10px; font: inherit; font-size: 14px; font-weight: 800; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 6px; }
.row2 { flex-direction: row; align-items: center; gap: 12px; width: 100%; font: inherit; text-align: left; color: var(--text); background: none; border-left: 0; border-right: 0; border-bottom: 0; }
.row2 .grow { flex: 1; display: flex; flex-direction: column; gap: 2px; }
.row2 b { font-size: 16px; }
.row2 em { font-size: 14px; }
.switch { position: relative; flex: none; width: 50px; height: 30px; background: var(--panel); border: 2px solid var(--edge); border-radius: 99px; }
.switch::after { content: ""; position: absolute; top: 3px; left: 3px; width: 20px; height: 20px; border-radius: 99px; background: var(--edge); }
.switch.on { background: var(--tint-dupes); }
.switch.on::after { left: 23px; background: #ffffff; }
.pick { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 60px; padding: 10px 14px; font: inherit; font-size: 16px; text-align: left; color: var(--text); background: var(--panel); border: 0; border-top: 2px solid var(--edge); }
.pick:first-child { border-top: 0; }
.pick.on { background: color-mix(in srgb, var(--tint-connections) 12%, var(--panel)); }
.radio { position: relative; flex: none; width: 26px; height: 26px; background: var(--panel); border: 2px solid var(--edge); border-radius: 99px; }
.radio.on::after { content: ""; position: absolute; inset: 4px; border-radius: 99px; background: var(--tint-connections); }
/* The answer sits over the button, where the eye already is. */
.line { margin: 0 2px 10px; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.about { margin: 14px 2px 0; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.note { margin: 14px 0 0; padding: 14px 16px; font-size: 15px; line-height: 1.4; background: var(--warn-bar); border: 2px solid var(--edge); border-radius: 8px; }
.good { color: var(--ok); font-weight: 700; }
.bad { color: var(--bad); font-weight: 700; }
.trust { display: flex; flex-direction: column; gap: 10px; margin-bottom: 12px; padding: 14px 16px; background: var(--warn-bar); border: 2px solid var(--edge); border-radius: 8px; }
.trust b { font-size: 16px; }
.trust p { margin: 0; font-size: 15px; line-height: 1.4; }
.trust code { font-size: 13px; overflow-wrap: anywhere; }
</style>
