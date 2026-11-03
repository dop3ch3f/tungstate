<!-- The clipboard, riding along while you browse to where it goes: what is
     held, and "Move here" once a folder that can take it is open. Shown on
     Files home too, so a held Move is never out of sight. -->
<script setup lang="ts">
import { computed } from "vue";
import { bytes } from "../../lib/format";
import { parentOf } from "../lib/paths";
import { useClipboard } from "../state/useClipboard";
import PButton from "../ui/PButton.vue";

const props = defineProps<{ folder: string | null; busy?: boolean }>();
defineEmits<{ paste: [] }>();
const board = useClipboard();

/** The clipboard came from this very folder: nothing to move or copy into. */
const fromHere = computed(() => {
  const held = board.clip.value;
  return !!held && !!props.folder && held.paths.length > 0 && held.paths.every((p) => parentOf(p) === props.folder);
});
const line = computed(() => {
  const held = board.clip.value!;
  if (!props.folder) return "Open a folder to put them in";
  return fromHere.value ? "They are in this folder; open another" : `${bytes(held.bytes)} from ${held.from}`;
});
</script>

<template>
  <div v-if="board.clip.value" class="clip">
    <button class="cancel" aria-label="Cancel" @click="board.drop()">✕</button>
    <div class="grow">
      <b>{{ board.clip.value.what }} to {{ board.clip.value.op }}</b>
      <span class="quiet">{{ line }}</span>
    </div>
    <PButton v-if="folder" class="paste-here" :busy="busy" :disabled="fromHere" @press="$emit('paste')">
      {{ board.clip.value.op === "copy" ? "Copy here" : "Move here" }}
    </PButton>
  </div>
</template>

<style scoped>
.clip { flex: none; display: flex; align-items: center; gap: 10px; margin: 0 12px 10px; padding: 8px; background: var(--panel); border: 2px solid var(--edge); border-radius: 10px; box-shadow: 4px 4px 0 var(--edge); }
.grow { flex: 1; min-width: 0; }
b { display: block; font-size: 15px; }
.quiet { font-size: 13px; color: var(--text-quiet); }
.cancel { flex: none; width: 44px; height: 44px; font: inherit; font-size: 20px; font-weight: 900; color: var(--text); background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.paste-here { width: auto; min-height: 48px; flex: none; font-size: 16px; white-space: nowrap; }
</style>
