<!-- Home. In the approved design it leads with how much this phone can free;
     that needs the photo library, which comes with Free up space, so until
     then the card says so and Home shows only what it really knows. -->
<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useConnections } from "../../state/useConnections";
import { usePhoneNav } from "../nav";
import PScreen from "../ui/PScreen.vue";
import PRow from "../ui/PRow.vue";
import PTile from "../ui/PTile.vue";

const conns = useConnections();
const nav = usePhoneNav();
onMounted(() => void conns.load());

const first = computed(() => conns.all.value.find((c) => c.scheme !== "fs") ?? null);
const others = computed(() => conns.all.value.filter((c) => c.scheme !== "fs").length - 1);
const line = computed(() => {
  const c = first.value;
  if (!c) return { title: "No connections yet", sub: "Add your NAS or a server to send photos to" };
  if (!c.last_check) return { title: c.name, sub: `${c.place} · not checked yet` };
  return c.last_check.ok
    ? { title: `${c.name} is reachable`, sub: others.value > 0 ? `${c.place} · and ${others.value} more` : c.place }
    : { title: `${c.name} could not be reached`, sub: c.last_check.note };
});
</script>

<template>
  <PScreen title="Tungstate">
    <div class="soon">
      <b>How much this phone can free</b>
      <p>Shows here once Tungstate can see your photos. That comes with the next build.</p>
    </div>
    <h2>Your connections</h2>
    <div class="list">
      <PRow :title="line.title" :sub="line.sub" chevron @press="nav.go('connections')">
        <template #lead><PTile art="connections" tint="var(--tint-connections)" light /></template>
      </PRow>
    </div>
  </PScreen>
</template>

<style scoped>
.soon { display: flex; flex-direction: column; gap: 6px; padding: 16px; border: 2px dashed var(--edge); border-radius: 8px; }
.soon b { font-size: 17px; }
.soon p { margin: 0; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
h2 { font-size: 15px; font-weight: 800; margin: 22px 0 8px; }
.list { background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; overflow: hidden; }
</style>
