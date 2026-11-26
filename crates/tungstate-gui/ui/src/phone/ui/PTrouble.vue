<!-- A failure in one plain sentence on the desktop's failure bar: coral,
     outlined in ink, the warning mark at text size beside the words. The
     engine's own words wait behind Details for when someone needs them (or
     is asked for them). No hard shadow: it is read, not pressed. -->
<script setup lang="ts">
import { computed, ref } from "vue";
import AlertIcon from "../../ui/AlertIcon.vue";
import { plainError } from "../lib/errors";

// `detail`: the words behind Details, when the sentence was written by the
// screen and the engine's own words are elsewhere (a run's failures).
const props = defineProps<{ error: unknown; where?: string; detail?: string }>();
const plain = computed(() => {
  const said = plainError(props.error, props.where);
  return props.detail ? { says: said.says, detail: props.detail } : said;
});
const open = ref(false);
</script>

<template>
  <div class="trouble" role="alert">
    <span class="mark"><AlertIcon tone="bad" /></span>
    <div class="words">
      <p class="says">{{ plain.says }}</p>
      <template v-if="plain.detail">
        <button class="more" :aria-expanded="open" @click="open = !open">{{ open ? "Hide details" : "Details" }}</button>
        <p v-if="open" class="detail">{{ plain.detail }}</p>
      </template>
    </div>
  </div>
</template>

<style scoped>
.trouble {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  margin: 0 0 12px;
  padding: 10px 12px;
  color: var(--warn-ink);
  background: var(--alarm-bar);
  border: 2px solid var(--warn-ink);
  border-radius: 8px;
  --alert-icon: var(--warn-ink);
  --alert-knock: var(--alarm-bar);
}
/* The desktop draws it at 16 beside 13pt text; 15pt here wants it larger. */
.mark { flex: none; display: flex; width: 20px; transform: scale(1.25); transform-origin: left top; }
.words { flex: 1; min-width: 0; }
.says { margin: 0; font-size: 15px; line-height: 1.4; font-weight: 700; }
.more { margin: 2px 0 -4px; padding: 6px 0; font: inherit; font-size: 14px; font-weight: 800; color: inherit; background: none; border: 0; text-decoration: underline; }
.more[aria-expanded="true"] { text-decoration: none; }
.detail { margin: 6px 0 0; padding: 8px 10px; font-family: ui-monospace, monospace; font-size: 12px; line-height: 1.4; overflow-wrap: anywhere; white-space: pre-wrap; color: var(--text-quiet); background: var(--panel); border: 2px solid var(--warn-ink); border-radius: 6px; -webkit-user-select: text; user-select: text; }
</style>
