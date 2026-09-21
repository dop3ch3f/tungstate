<!-- The front door. One button, and it does not require registering anything.
     That inversion is the point of the slice: `learn_folder` and
     `compare_folder` take a bare path, so somebody can be shown their own
     files before they have learned a word of this app's vocabulary. -->
<script setup lang="ts">
import { onMounted } from "vue";
import { useFolders } from "../../state/useFolders";
import Button from "../../ui/Button.vue";
import Notice from "../../ui/Notice.vue";
import Empty from "../../ui/Empty.vue";

const f = useFolders();
onMounted(() => f.listRegistered());
</script>

<template>
  <div class="start">
    <div class="column">
      <h1>Tidy a folder</h1>
      <p class="intro">
        Point at one and you will see how it is filed now, beside what every
        other way of filing would do to it. Nothing moves until you say so.
      </p>

      <Button look="primary" @click="f.pick()">Point at a folder…</Button>

      <Notice tone="bad" v-if="f.problem.value">{{ f.problem.value }}</Notice>

      <section class="known" v-if="f.registered.value.length">
        <h2>Folders you have already</h2>
        <button
          v-for="folder in f.registered.value"
          :key="folder.root"
          class="known-row"
          @click="folder.broken ? f.look(folder.root) : f.open(folder.root)"
        >
          <span class="known-name">{{ folder.name }}</span>
          <span class="path known-path">{{ folder.root }}</span>
          <span class="known-state" v-if="folder.broken">rules will not load</span>
          <span class="known-state dim" v-else-if="!folder.has_rules">no rules yet</span>
        </button>
      </section>

      <Empty v-else line="No folders here yet. Downloads is usually the messiest one." />
    </div>
  </div>
</template>

<style scoped>
.start { position: absolute; inset: 0; overflow-y: auto; scrollbar-gutter: stable; }
.column { max-width: 720px; margin: 0 auto; padding: 56px var(--s6) var(--s6); }

h1 { font-size: var(--display); font-weight: 600; margin: 0; letter-spacing: -0.01em; }
.intro {
  font-size: var(--body);
  line-height: 1.6;
  color: var(--text-quiet);
  margin: var(--s3) 0 var(--s5);
  max-width: 62ch;
}

.known { margin-top: 44px; }
.known h2 { font-size: var(--small); font-weight: 600; margin: 0 0 var(--s2); }
.known-row {
  display: flex;
  align-items: baseline;
  gap: var(--s3);
  width: calc(100% + var(--s3) * 2);
  margin: 0 calc(var(--s3) * -1);
  padding: var(--s2) var(--s3);
  font: inherit;
  text-align: left;
  background: none;
  border: none;
  border-radius: var(--radius);
  color: inherit;
  cursor: pointer;
}
.known-row:hover { background: var(--surface-hover); }
.known-name { font-weight: 600; flex: none; }
.known-path { color: var(--text-faint); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.known-state { margin-left: auto; flex: none; font-size: var(--fine); color: var(--bad); }
.known-state.dim { color: var(--text-faint); }
</style>
