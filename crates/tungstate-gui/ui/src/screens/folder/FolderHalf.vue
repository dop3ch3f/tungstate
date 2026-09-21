<!-- One flow, four states: point at a folder, read it, pick a way of filing,
     see what that would do. The way back is on every screen after the first. -->
<script setup lang="ts">
import { ref, watch } from "vue";
import { useFolders } from "../../state/useFolders";
import FolderStart from "./FolderStart.vue";
import FolderRead from "./FolderRead.vue";
import FolderPreview from "./FolderPreview.vue";
import Working from "../../ui/Working.vue";
import Button from "../../ui/Button.vue";

const f = useFolders();
const chosen = ref<string | null>(null);

// A new folder must not inherit the last one's choice.
watch(() => f.root.value, () => (chosen.value = null));
</script>

<template>
  <div class="half">
    <div class="pending" v-if="f.phase.value === 'reading'">
      <Working
        what="Reading the folder"
        note="every way of filing is planned against it, which takes a moment on a large one"
      />
    </div>

    <FolderStart v-else-if="f.phase.value === 'start'" />
    <FolderRead v-else-if="f.phase.value === 'choosing'" v-model:chosen="chosen" />
    <FolderPreview v-else />

    <div class="back" v-if="f.phase.value !== 'start'">
      <Button look="link" @click="f.back()">All folders</Button>
    </div>
  </div>
</template>

<style scoped>
.half { position: absolute; inset: 0; }
.pending { display: grid; place-items: center; height: 100%; }
.back { position: absolute; top: var(--s5); right: var(--s6); }
</style>
