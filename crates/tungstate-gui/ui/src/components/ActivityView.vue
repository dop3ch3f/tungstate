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
  <header><h1>Activity</h1></header>

  <p class="lede">
    Everything tungstate has done, newest first. Search a full path to see only that file.
  </p>

  <form class="row" style="margin-bottom:18px" @submit.prevent="search">
    <div class="field">
      <label for="q">Path</label>
      <input id="q" v-model="query" placeholder="/Users/you/Videos/holiday.mp4" />
    </div>
    <button class="act" type="submit">Search</button>
    <button v-if="searched" class="act" type="button" @click="load">Show everything</button>
  </form>

  <div v-if="error" class="notice bad">{{ error }}</div>

  <div v-if="!ops.length" class="empty">
    <strong>Nothing here yet.</strong>
    Once you move some files, every one of them will be listed here with where it went.
  </div>

  <table v-else class="ledger">
    <thead>
      <tr><th>When</th><th>File</th><th style="text-align:right">Size</th><th>State</th></tr>
    </thead>
    <tbody>
      <tr v-for="op in ops" :key="op.id">
        <td class="size" style="text-align:left">{{ when(op.started_at) }}</td>
        <td class="path">
          {{ op.destination ?? op.source }}
          <div v-if="op.source && op.destination" class="detail">from {{ op.source }}</div>
          <div v-if="op.note" class="detail">{{ op.note }}</div>
        </td>
        <td class="size">{{ op.size === null ? "—" : bytes(op.size) }}</td>
        <td class="state" :class="op.status">{{ op.status }}</td>
      </tr>
    </tbody>
  </table>
</template>
