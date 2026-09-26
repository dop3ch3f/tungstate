<!-- What putting a run back did, said once wherever it was asked from. -->
<script setup lang="ts">
import { useSync } from "../../state/useSync";
import { files, syncPutBack, syncUnsent } from "../../lib/counts";
import Notice from "../../ui/Notice.vue";

const s = useSync();
</script>

<template>
  <Notice v-if="s.undone.value">
    Put back: {{ files(syncPutBack(s.undone.value)) }} moved back where they were,
    {{ files(syncUnsent(s.undone.value)) }} it had copied set aside again.
    <template v-for="r in s.undone.value.revived" :key="`${r.member}${r.path}`">
      The next run brings {{ r.path }} back to {{ r.member }}.
    </template>
    <template v-if="s.undone.value.parked_left">
      {{ s.undone.value.parked_left }} conflicting {{ s.undone.value.parked_left === 1 ? "version it parked is" : "versions it parked are" }}
      still in the set-aside area.
    </template>
  </Notice>
</template>
