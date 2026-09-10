<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api, type Link } from "../api";

const emit = defineEmits<{ run: [name: string] }>();

const links = ref<Link[]>([]);
const error = ref("");
const creating = ref(false);

const form = ref({
  name: "",
  source: "",
  destination: "",
  source_policy: "delete",
  verify: "hash",
  order: "largest-first",
  on_conflict: "quarantine",
  cooldown_secs: 30,
});

async function load() {
  try { links.value = await api.links(); error.value = ""; }
  catch (e) { error.value = String(e); }
}

async function create() {
  try {
    await api.createLink(form.value as unknown as Link);
    creating.value = false;
    form.value.name = form.value.source = form.value.destination = "";
    await load();
  } catch (e) { error.value = String(e); }
}

async function start(name: string) {
  try { await api.run(name); emit("run", name); }
  catch (e) { error.value = String(e); }
}

onMounted(load);
</script>

<template>
  <header>
    <h1>Folders</h1>
    <button class="act" @click="creating = !creating">
      {{ creating ? "Cancel" : "Add a pair" }}
    </button>
  </header>

  <div v-if="error" class="notice bad">{{ error }}</div>

  <form v-if="creating" @submit.prevent="create">
    <div class="field">
      <label for="name">Name</label>
      <input id="name" v-model="form.name" placeholder="laptop-to-nas" required />
      <span class="hint">What you will call this pair of folders.</span>
    </div>

    <div class="field">
      <label for="src">Move files out of</label>
      <input id="src" v-model="form.source" placeholder="/Users/you/Videos" required />
    </div>

    <div class="field">
      <label for="dst">And into</label>
      <input id="dst" v-model="form.destination" placeholder="/Volumes/nas/inbox" required />
    </div>

    <div class="grid2">
      <div class="field">
        <label for="policy">The originals</label>
        <select id="policy" v-model="form.source_policy">
          <option value="delete">Delete once the copy is verified</option>
          <option value="trash">Move to the trash</option>
          <option value="keep">Leave alone</option>
        </select>
      </div>
      <div class="field">
        <label for="verify">Check each copy by</label>
        <select id="verify" v-model="form.verify">
          <option value="hash">Comparing the fingerprint taken while copying</option>
          <option value="readback">Reading it back and comparing again</option>
          <option value="size">Size only</option>
        </select>
      </div>
      <div class="field">
        <label for="order">Take files</label>
        <select id="order" v-model="form.order">
          <option value="largest-first">Largest first, to free space soonest</option>
          <option value="smallest-first">Smallest first</option>
          <option value="oldest-first">Oldest first</option>
          <option value="discovered">In folder order</option>
        </select>
      </div>
      <div class="field">
        <label for="conflict">If a name is already taken and nobody answers</label>
        <select id="conflict" v-model="form.on_conflict">
          <option value="quarantine">Set it aside at the destination</option>
          <option value="rename">Keep both, renaming the new one</option>
          <option value="skip">Leave it on this machine</option>
        </select>
      </div>
    </div>

    <p class="hint" style="max-width:62ch;margin:14px 0">
      Files written in the last {{ form.cooldown_secs }} seconds are left for the next run,
      in case something is still writing them.
    </p>
    <button class="act primary" type="submit">Create</button>
  </form>

  <template v-else>
    <div v-if="!links.length" class="empty">
      <strong>No folder pairs yet.</strong>
      A pair is somewhere files come from, somewhere they go, and what should happen
      to the originals once each copy is checked.
    </div>

    <div v-for="link in links" :key="link.name" class="link-row">
      <div class="name">{{ link.name }}</div>
      <div class="route">{{ link.source }} → {{ link.destination }}</div>
      <div class="rules">
        Originals: {{ link.source_policy === "delete" ? "deleted once verified"
          : link.source_policy === "trash" ? "moved to trash" : "kept" }}
        · checked by {{ link.verify }} · {{ link.order }}
      </div>
      <button class="act primary go" @click="start(link.name)">Run</button>
    </div>
  </template>
</template>
