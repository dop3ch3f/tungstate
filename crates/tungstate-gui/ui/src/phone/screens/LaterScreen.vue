<!-- A tab whose screen is not built yet, saying so in plain words rather than
     showing a screen that looks finished and does nothing. -->
<script setup lang="ts">
import { computed } from "vue";
import type { Tab } from "../nav";
import PScreen from "../ui/PScreen.vue";
import PTile from "../ui/PTile.vue";
import type { Art } from "../../lib/pixels";

const props = defineProps<{ tab: Tab }>();

const ABOUT: Partial<Record<Tab, { title: string; art: Art; tint: string; does: string; needs: string }>> = {
  dupes: {
    title: "Duplicates",
    art: "dupes",
    tint: "var(--tint-dupes)",
    does: "Find copies and look-alikes in your photos, keep the best of each, and clear the rest.",
    needs: "Comes after Free up space, on the same photo access.",
  },
  organise: {
    title: "Organise",
    art: "folder",
    tint: "var(--tint-folder)",
    does: "Tidy Downloads and other folders into a clean layout. Every move can be put back.",
    needs: "Comes once Tungstate can open folders on this phone.",
  },
};
const about = computed(() => ABOUT[props.tab]!);
</script>

<template>
  <PScreen :title="about.title">
    <div class="card">
      <PTile :art="about.art" :tint="about.tint" light :size="48" />
      <p class="does">{{ about.does }}</p>
      <p class="needs">{{ about.needs }}</p>
    </div>
  </PScreen>
</template>

<style scoped>
.card { display: flex; flex-direction: column; gap: 12px; padding: 18px; background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; }
.does { margin: 0; font-size: 17px; line-height: 1.4; font-weight: 600; }
.needs { margin: 0; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
</style>
