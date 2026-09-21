<!-- Mounted once, at the top of the window. Draws whatever `ask()` is waiting
     on, and nothing when nothing is. -->
<script setup lang="ts">
import Sheet from "./Sheet.vue";
import Button from "./Button.vue";
import { dialog, answer } from "./useDialog";
</script>

<template>
  <Sheet v-if="dialog.open.value" @dismiss="answer(null)">
    <h2 class="title">{{ dialog.open.value.title }}</h2>
    <p class="blurb" v-if="dialog.open.value.why">{{ dialog.open.value.why }}</p>
    <ul class="particulars" v-if="dialog.open.value.detail?.length">
      <li v-for="line in dialog.open.value.detail" :key="line">{{ line }}</li>
    </ul>
    <label class="optin" v-if="dialog.open.value.checkbox">
      <input type="checkbox" v-model="dialog.checked.value" />
      {{ dialog.open.value.checkbox }}
    </label>
    <div class="choices">
      <Button
        v-for="choice in dialog.open.value.choices"
        :key="choice.id"
        :look="choice.look ?? 'plain'"
        @click="answer(choice.id)"
      >{{ choice.label }}</Button>
    </div>
  </Sheet>
</template>

<style scoped>
.title { font-size: var(--body); font-weight: 600; margin: 0 0 var(--s2); }
.blurb { font-size: var(--small); color: var(--text-quiet); margin: 0; line-height: 1.55; }
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
.choices { display: flex; justify-content: flex-end; gap: var(--s2); margin-top: var(--s5); }
</style>
