<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api, bytes, when, type Op } from "../api";

const ops = ref<Op[]>([]);
const query = ref("");
const error = ref("");
const searched = ref(false);

async function load() {
  try { ops.value = await api.recent(); searched.value = false; error.value = ""; }
  catch (e) { error.value = String(e); }
}

async function search() {
  if (!query.value.trim()) return load();
  try {
    ops.value = await api.history(query.value.trim());
    searched.value = true;
    error.value = "";
  } catch (e) { error.value = String(e); }
}

onMounted(load);
</script>

<template>
  <!-- `.sheet` is what makes a view a view: it is the flex child that takes the
       remaining height, scrolls, and carries the padding. Without it this
       component's elements became flex items of the frame itself, so the page
       had no padding, the heading no style, and the table could not scroll. -->
  <div class="sheet">
    <h1>Activity</h1>
    <p class="sub">
      Everything tungstate has done, newest first. Search a full path to see only that file.
    </p>

    <form class="seek" @submit.prevent="search">
      <label class="opt" for="q">
        <span>Path</span>
        <input id="q" type="text" v-model="query" placeholder="/Users/you/Videos/holiday.mp4" />
      </label>
      <button class="btn primary" type="submit">Search</button>
      <button v-if="searched" class="btn" type="button" @click="load">Show everything</button>
    </form>

    <div v-if="error" class="notice bad">{{ error }}</div>

    <div v-if="!ops.length" class="empty">
      <strong>{{ searched ? "Nothing matches that path." : "Nothing here yet." }}</strong>
      {{ searched
        ? "Search the whole path a file had, exactly as it was when it moved."
        : "Once you move some files, every one of them will be listed here with where it went." }}
    </div>

    <table v-else class="ledger">
      <thead>
        <tr><th>When</th><th>File</th><th class="num">Size</th><th>State</th></tr>
      </thead>
      <tbody>
        <tr v-for="op in ops" :key="op.id">
          <td class="when">{{ when(op.started_at) }}</td>
          <td class="path">
            {{ op.destination ?? op.source }}
            <div v-if="op.source && op.destination" class="detail">from {{ op.source }}</div>
            <div v-if="op.note" class="detail">{{ op.note }}</div>
          </td>
          <td class="num">{{ op.size === null ? "—" : bytes(op.size) }}</td>
          <td class="state" :class="op.status">{{ op.status }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>
