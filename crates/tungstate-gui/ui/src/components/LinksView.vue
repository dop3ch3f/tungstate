<script setup lang="ts">
import { ref } from "vue";
import { api, CHOICES, type Link, type NewLink } from "../api";

const props = defineProps<{ links: Link[]; busy: boolean; places: string[] }>();
const emit = defineEmits<{
  changed: [];
  run: [name: string];
  preview: [name: string];
}>();

const error = ref("");
const note = ref("");
const confirming = ref<Link | null>(null);
const working = ref("");

/// A saved pair made without moving anything, which is what `link add` does
/// on the command line and what the window could not do at all.
const adding = ref(false);
const blank = (): NewLink => ({
  name: "",
  source: "",
  destination: "",
  source_policy: "delete",
  verify: "hash",
  order: "largest-first",
  on_conflict: "quarantine",
  cooldown_secs: 30,
});
const form = ref<NewLink>(blank());

async function create() {
  error.value = "";
  note.value = "";
  working.value = "#new";
  try {
    await api.createLink(form.value);
    note.value = `Saved “${form.value.name}”. Nothing has moved; run it when you are ready.`;
    adding.value = false;
    form.value = blank();
    emit("changed");
  } catch (e) {
    error.value = String(e);
  } finally {
    working.value = "";
  }
}

/** The word for what a setting means, rather than the token stored. */
function label(group: keyof typeof CHOICES, value: string): string {
  const found = CHOICES[group].find(([token]) => token === value);
  return found ? found[1] : value;
}

function cooldown(seconds: number): string {
  if (seconds === 0) return "no wait";
  if (seconds < 60) return `${seconds}s settled`;
  if (seconds < 3600) return `${Math.round(seconds / 60)}m settled`;
  return `${Math.round(seconds / 3600)}h settled`;
}

async function remove(link: Link) {
  working.value = link.name;
  error.value = "";
  note.value = "";
  try {
    const retired = await api.removeLink(link.name);
    // Said out loud rather than implied. The row is still there when a link
    // has run, because the history refers to it, and somebody reading the
    // activity later should not be surprised to still see the name.
    note.value = retired
      ? `Removed “${link.name}”. It had run before, so its record is kept and Activity still resolves it. The name is free to use again.`
      : `Removed “${link.name}”.`;
    confirming.value = null;
    emit("changed");
  } catch (e) {
    error.value = String(e);
  } finally {
    working.value = "";
  }
}
</script>

<template>
  <div class="sheet">
    <div style="display:flex; align-items:baseline; justify-content:space-between; gap:12px">
      <h1>Links</h1>
      <div class="go">
        <button class="btn primary" @click="adding = true; form = blank()">Add a link</button>
      </div>
    </div>
    <p class="sub">
      Saved pairs you run again: a source, a destination, and what should happen to the
      originals. A one-off move from Browse makes one of these too, unless you named it.
    </p>

    <div v-if="error" class="notice bad">{{ error }}</div>
    <div v-if="note" class="notice">{{ note }}</div>

    <div v-if="!props.links.length" class="empty">
      <strong>No links yet.</strong>
      <span>
        Tick some files in Browse, choose Move or Copy, and give the pair a name. It appears
        here, ready to run again without picking anything.
      </span>
    </div>

    <table v-else class="ledger links">
      <thead>
        <tr>
          <th>Name</th>
          <th>From</th>
          <th>To</th>
          <th>Settings</th>
          <th class="actions"></th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="l in props.links" :key="l.name">
          <td><b>{{ l.name }}</b></td>
          <td class="addr">{{ l.source }}</td>
          <td class="addr">{{ l.destination }}</td>
          <td>
            <!-- Every setting, because the whole reason this screen exists is
                 that order and cooldown were unreachable and unreadable. -->
            <div class="detail">{{ label("source_policy", l.source_policy) }}</div>
            <div class="detail">
              {{ label("verify", l.verify) }} · {{ label("order", l.order) }}
            </div>
            <div class="detail">
              {{ label("on_conflict", l.on_conflict) }} · {{ cooldown(l.cooldown_secs) }}
            </div>
          </td>
          <td class="actions">
            <button class="btn quiet small" :disabled="props.busy"
                    @click="emit('preview', l.name)">Preview</button>
            <button class="btn quiet small" :disabled="props.busy"
                    @click="emit('run', l.name)">Run</button>
            <button class="btn quiet small" :disabled="working === l.name"
                    @click="confirming = l">Remove</button>
          </td>
        </tr>
      </tbody>
    </table>

    <div v-if="adding" class="veil" @click.self="adding = false">
      <div class="modal" role="dialog" aria-modal="true">
        <div class="cap">
          <h2>A saved pair</h2>
          <p class="why">
            Two folders and what should happen between them. Nothing moves until you run it.
            A connection is written <code>name:folder</code>, the same way it is typed on the
            command line.
          </p>
        </div>
        <div class="body">
          <div class="opt">
            <label for="lname">Called</label>
            <input id="lname" v-model="form.name" autocomplete="off" placeholder="laptop to nas" />
          </div>
          <div class="pair">
            <div class="opt">
              <label for="lsrc">From</label>
              <input id="lsrc" v-model="form.source" autocomplete="off" spellcheck="false"
                     list="link-places" placeholder="/Users/you/Movies" />
            </div>
            <div class="opt">
              <label for="ldst">To</label>
              <input id="ldst" v-model="form.destination" autocomplete="off" spellcheck="false"
                     list="link-places" placeholder="nas:inbox" />
            </div>
          </div>
          <datalist id="link-places">
            <option v-for="p in props.places" :key="p" :value="p" />
          </datalist>

          <div class="opt">
            <label for="lpolicy">What happens to the originals</label>
            <select id="lpolicy" v-model="form.source_policy">
              <option v-for="[token, said] in CHOICES.source_policy" :key="token" :value="token">
                {{ said }}
              </option>
            </select>
          </div>
          <div class="pair">
            <div class="opt">
              <label for="lverify">Check each copy by</label>
              <select id="lverify" v-model="form.verify">
                <option v-for="[token, said] in CHOICES.verify" :key="token" :value="token">
                  {{ said }}
                </option>
              </select>
            </div>
            <div class="opt">
              <label for="lorder">Take files</label>
              <select id="lorder" v-model="form.order">
                <option v-for="[token, said] in CHOICES.order" :key="token" :value="token">
                  {{ said }}
                </option>
              </select>
            </div>
          </div>
          <div class="pair">
            <div class="opt">
              <label for="lconflict">If a name is taken</label>
              <select id="lconflict" v-model="form.on_conflict">
                <option v-for="[token, said] in CHOICES.on_conflict" :key="token" :value="token">
                  {{ said }}
                </option>
              </select>
            </div>
            <div class="opt">
              <label for="lcool">Leave alone if written within</label>
              <select id="lcool" v-model.number="form.cooldown_secs">
                <option :value="0">no wait</option>
                <option :value="30">30 seconds</option>
                <option :value="300">5 minutes</option>
                <option :value="3600">1 hour</option>
              </select>
              <span class="note">Skips anything still being written, and takes it next run.</span>
            </div>
          </div>
        </div>
        <div class="feet">
          <span class="spacer"></span>
          <button class="btn" @click="adding = false">Cancel</button>
          <button class="btn primary"
                  :disabled="working === '#new' || !form.name.trim() || !form.source.trim() || !form.destination.trim()"
                  @click="create">Save it</button>
        </div>
      </div>
    </div>

    <div v-if="confirming" class="veil" @click.self="confirming = null">
      <div class="modal" role="dialog" aria-modal="true">
        <div class="cap">
          <h2>Remove {{ confirming.name }}?</h2>
          <p class="why">
            Nothing is moved or deleted on either side. This only forgets the pair, so it
            stops being offered. Anything it already moved stays exactly where it is, and
            Activity still shows what it did.
          </p>
        </div>
        <div class="feet">
          <span class="spacer"></span>
          <button class="btn" @click="confirming = null">Keep it</button>
          <button class="btn primary" :disabled="working === confirming.name"
                  @click="remove(confirming)">Remove</button>
        </div>
      </div>
    </div>
  </div>
</template>
