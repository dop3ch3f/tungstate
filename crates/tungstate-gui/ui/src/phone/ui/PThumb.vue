<!-- A file's preview: the picture where there is one, its kind's mark where
     there is not (yet). The mark is drawn first and the picture laid over it
     once it arrives, so a slow NAS never leaves a hole in the list. -->
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { filePicture } from "../../engine/pictures";
import { artOf, hasPicture, tintOf } from "../lib/look";
import PTile from "./PTile.vue";

const props = withDefaults(defineProps<{ name: string; path: string; isDir?: boolean; size?: number; fill?: boolean }>(), {
  isDir: false,
  size: 44,
  fill: false,
});

const failed = ref(false);
const loaded = ref(false);
watch(() => props.path, () => {
  failed.value = false;
  loaded.value = false;
});
const entry = computed(() => ({ name: props.name, is_dir: props.isDir, path: props.path }));
const src = computed(() => (!props.isDir && hasPicture(entry.value) && !failed.value ? filePicture(props.path) : null));
const video = computed(() => artOf(entry.value) === "video");
</script>

<template>
  <span class="thumb" :class="{ fill: fill }" :style="fill ? undefined : { width: `${size}px`, height: `${size}px` }">
    <PTile v-if="!loaded" class="mark" :art="artOf(entry)" :tint="tintOf(entry)" :size="fill ? 64 : size" />
    <img v-if="src" :src="src" alt="" loading="lazy" decoding="async" :class="{ on: loaded }" @load="loaded = true" @error="failed = true" />
    <span v-if="video && loaded" class="play" aria-hidden="true">▶</span>
  </span>
</template>

<style scoped>
.thumb { position: relative; flex: none; display: grid; place-items: center; overflow: hidden; border-radius: 6px; }
.thumb.fill { width: 100%; aspect-ratio: 1; border: 2px solid var(--edge); background: var(--panel); box-shadow: 3px 3px 0 var(--edge); }
.thumb.fill .mark { border: 0; border-radius: 0; width: 100% !important; height: 100% !important; }
img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; opacity: 0; border: 2px solid var(--edge); border-radius: 6px; }
.fill img { border: 0; border-radius: 0; }
img.on { opacity: 1; }
.play { position: absolute; left: 4px; bottom: 4px; padding: 0 5px; font-size: 10px; line-height: 16px; color: #fdfdf8; background: #151515; border-radius: 3px; }
</style>
