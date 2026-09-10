<script setup lang="ts">
defineEmits<{ begin: [] }>();
</script>

<template>
  <div class="welcome">
    <div class="card">
      <h1>Move files without holding your breath</h1>
      <p>
        Tungstate copies one file at a time, checks it arrived intact, and only then
        removes the original. Close the lid halfway through and nothing is lost:
        run it again and it picks up where it stopped.
      </p>
      <ol>
        <li><b>Pick two folders.</b> One on this machine, one on your NAS or drive.</li>
        <li><b>Choose what to move.</b> Select files on either side.</li>
        <li><b>Move or copy.</b> Watch each file being verified as it lands.</li>
      </ol>
      <div class="go">
        <button class="btn primary" @click="$emit('begin')">Choose folders</button>
        <span style="color: var(--dust-dim); font-size: 12px">Nothing is deleted until its copy is verified.</span>
      </div>
    </div>
  </div>
</template>
