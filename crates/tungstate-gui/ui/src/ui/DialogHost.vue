<!-- Mounted once, at the top of the window. Draws whatever `ask()` is waiting
     on, and nothing when nothing is. -->
<script setup lang="ts">
import Sheet from "./Sheet.vue";
import Button from "./Button.vue";
import ActionBar from "./ActionBar.vue";
import { dialog, answer } from "./useDialog";
import { useNav } from "../nav";

// A question wears the tile of the section that asked it; without this every
// dialog showed the default, which is Settings.
const nav = useNav();
</script>

<template>
  <Sheet v-if="dialog.open.value" :of="nav.view.value" :title="dialog.open.value.title" @dismiss="answer(null)">
    <p class="blurb" v-if="dialog.open.value.why">{{ dialog.open.value.why }}</p>
    <ul class="particulars" v-if="dialog.open.value.detail?.length">
      <li v-for="line in dialog.open.value.detail" :key="line">{{ line }}</li>
    </ul>
    <label class="optin" v-if="dialog.open.value.checkbox">
      <input type="checkbox" v-model="dialog.checked.value" />
      {{ dialog.open.value.checkbox }}
    </label>
    <ActionBar>
      <Button
        v-for="choice in dialog.open.value.choices"
        :key="choice.id"
        :look="choice.look ?? 'plain'"
        @click="answer(choice.id)"
      >{{ choice.label }}</Button>
    </ActionBar>
  </Sheet>
</template>

<style scoped>
.blurb { font-size: var(--small); color: var(--text-quiet); margin: 0; line-height: 1.55; }
/* Room between what the question says and the bar that answers it. */
.blurb, .particulars, .optin { margin-bottom: var(--s4); }
.particulars { margin: var(--s3) 0 0; padding-left: var(--s4); font-size: var(--small); color: var(--text-quiet); }
.particulars li { margin-bottom: var(--s1); }
.optin {
  display: flex;
  gap: var(--s2);
  align-items: flex-start;
  font-size: var(--small);
  color: var(--text-quiet);
  margin-top: var(--s3);
}
</style>
