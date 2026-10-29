<!-- Home: how much this phone can free, then your connections. The number
     is Free up space's own, so the two screens never disagree; until the
     person lets the app see their photos, Home asks for that instead. -->
<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useConnections } from "../../state/useConnections";
import { bytes, plural } from "../../lib/format";
import { usePhotos } from "../state/usePhotos";
import PButton from "../ui/PButton.vue";
import { usePhoneNav } from "../nav";
import PScreen from "../ui/PScreen.vue";
import PRow from "../ui/PRow.vue";
import PTile from "../ui/PTile.vue";

const conns = useConnections();
const nav = usePhoneNav();
const lib = usePhotos();
onMounted(() => {
  void conns.load();
  void lib.load();
});

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
    <div v-if="lib.canRead.value" class="hero">
      <span class="big">{{ bytes(lib.chosen.value.bytes) }}</span>
      <span class="sub">{{ lib.loading.value ? "counting…" : `you can free · ${plural(lib.chosen.value.count, "photo and video", "photos and videos")} older than 3 months` }}</span>
    </div>
    <div v-else-if="lib.access.value" class="soon">
      <b>How much this phone can free</b>
      <p>Shows here once Tungstate can see your photos and videos.</p>
    </div>
    <h2>Your connections</h2>
    <div class="list">
      <PRow :title="line.title" :sub="line.sub" chevron @press="nav.go('connections')">
        <template #lead><PTile art="connections" tint="var(--tint-connections)" light /></template>
      </PRow>
    </div>
    <template #act>
      <PButton v-if="lib.canRead.value && lib.chosen.value.count" @press="nav.go('free')">Free {{ bytes(lib.chosen.value.bytes) }}</PButton>
      <PButton v-else-if="lib.access.value === 'ask'" @press="nav.go('free')">See what you can free</PButton>
    </template>
  </PScreen>
</template>

<style scoped>
.hero { display: flex; flex-direction: column; gap: 8px; margin: 6px 0 4px; }
.big { font-size: 52px; font-weight: 800; letter-spacing: -1.5px; line-height: 0.95; }
.sub { font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
.soon { display: flex; flex-direction: column; gap: 6px; padding: 16px; border: 2px dashed var(--edge); border-radius: 8px; }
.soon b { font-size: 17px; }
.soon p { margin: 0; font-size: 15px; line-height: 1.4; color: var(--text-quiet); }
h2 { font-size: 15px; font-weight: 800; margin: 22px 0 8px; }
.list { background: var(--panel); border: 2px solid var(--edge); border-radius: 8px; overflow: hidden; }
</style>
