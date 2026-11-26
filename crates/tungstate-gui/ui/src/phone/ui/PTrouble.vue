<!-- A failure in one plain sentence, with the engine's own words behind
     Details for when someone needs them (or is asked for them). -->
<script setup lang="ts">
import { computed, ref } from "vue";
import { plainError } from "../lib/errors";

const props = defineProps<{ error: unknown; where?: string }>();
const plain = computed(() => plainError(props.error, props.where));
const open = ref(false);
</script>

<template>
  <div class="trouble" role="alert">
    <p class="says">{{ plain.says }}</p>
    <template v-if="plain.detail">
      <button class="more" :aria-expanded="open" @click="open = !open">{{ open ? "Hide details" : "Details" }}</button>
      <p v-if="open" class="detail">{{ plain.detail }}</p>
    </template>
  </div>
</template>

<style scoped>
.trouble { margin: 0 2px 10px; }
.says { margin: 0; font-size: 15px; line-height: 1.4; font-weight: 700; color: var(--bad); }
.more { margin-top: 4px; padding: 6px 0; font: inherit; font-size: 14px; font-weight: 800; color: var(--text); background: none; border: 0; text-decoration: underline; }
.more[aria-expanded="true"] { text-decoration: none; }
.detail { margin: 4px 0 0; padding: 10px 12px; font-family: ui-monospace, monospace; font-size: 12px; line-height: 1.4; overflow-wrap: anywhere; color: var(--text-quiet); background: var(--panel); border: 2px solid var(--edge); border-radius: 6px; -webkit-user-select: text; user-select: text; }
</style>
