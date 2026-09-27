<!-- "Stop after these files", then "Stop now" in the same place. The second
     is the one that loses work, and it appears exactly where the first was
     clicked, so it ignores clicks for a moment: a double click on the gentle
     stop must not become the harsh one. -->
<script setup lang="ts">
import { watch } from "vue";
import { holdoff } from "../lib/holdoff";
import Button from "./Button.vue";

const props = defineProps<{ stopping: boolean; halting: boolean }>();
const emit = defineEmits<{ stop: []; now: [] }>();
const settling = holdoff(600);
watch(
  () => props.stopping,
  (on) => on && settling.start(),
);
const now = () => settling.ready() && emit("now");
</script>

<template>
  <Button v-if="!stopping" @click="emit('stop')">Stop after these files</Button>
  <Button v-else-if="!halting" look="danger" @click="now()">Stop now</Button>
</template>
