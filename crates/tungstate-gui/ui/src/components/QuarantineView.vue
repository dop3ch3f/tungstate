<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api, type Link } from "../api";

const links = ref<Link[]>([]);
const chosen = ref("");
const files = ref<string[]>([]);
const error = ref("");

async function load() {
  try {
    links.value = await api.links();
    if (!chosen.value && links.value.length) chosen.value = links.value[0].name;
    if (chosen.value) files.value = await api.quarantined(chosen.value);
    error.value = "";
  } catch (e) { error.value = String(e); }
}

onMounted(load);
</script>

<template>
  <header><h1>Quarantine</h1></header>

  <p class="lede">
    When a name was already taken and nobody was here to ask, the arriving file was set
    aside rather than overwriting anything. Nothing here has been lost.
  </p>

  <div v-if="error" class="notice bad">{{ error }}</div>

  <div v-if="links.length" class="field" style="max-width:340px">
    <label for="link">Folder pair</label>
    <select id="link" v-model="chosen" @change="load">
      <option v-for="l in links" :key="l.name" :value="l.name">{{ l.name }}</option>
    </select>
  </div>

  <div v-if="!files.length" class="empty">
    <strong>Nothing set aside.</strong>
    Every file that moved found a free name at the destination.
  </div>

  <template v-else>
    <p class="lede">
      These are in the destination folder, under
      <code style="font-family:var(--mono)">.tungstate-quarantine</code>.
      Move or delete them there once you have decided which copy you want.
    </p>
    <table class="ledger">
      <thead><tr><th>File</th></tr></thead>
      <tbody>
        <tr v-for="f in files" :key="f"><td class="path">{{ f }}</td></tr>
      </tbody>
    </table>
  </template>
</template>
