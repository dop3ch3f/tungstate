<!-- The one button. Variants rather than a new set of classes per screen.
     `plain` is the default; `primary` is filled and there is at most one per
     screen; `danger` is for the two places that lose something. -->
<script setup lang="ts">
const props = withDefaults(
  defineProps<{ look?: "plain" | "primary" | "danger" | "link" }>(),
  { look: "plain" },
);
// A lookup rather than an interpolated class: `scripts/check-css.mjs` cannot
// follow a computed name, so a backtick binding is an error there.
const LOOK = {
  plain: "control-plain",
  primary: "control-primary",
  danger: "control-danger",
  link: "control-link",
} as const;
</script>

<template>
  <button class="control" :class="LOOK[props.look]"><slot /></button>
</template>

<style scoped>
.control {
  font: inherit;
  font-size: var(--small);
  color: var(--text-quiet);
  background: none;
  border: 1px solid var(--edge);
  border-radius: var(--radius);
  padding: var(--s2) var(--s3);
  cursor: pointer;
  transition: background var(--quick) var(--ease), border-color var(--quick) var(--ease);
}
.control:hover:not(:disabled) { color: var(--text); border-color: var(--text-faint); }
.control:disabled { opacity: 0.45; cursor: default; }

.control-plain { /* the default, already drawn above */ }

.control-primary {
  font-size: var(--body);
  font-weight: 500;
  color: var(--control-ink);
  background: var(--control);
  border-color: transparent;
  border-radius: var(--radius-lg);
  padding: 10px 18px;
}
.control-primary:hover:not(:disabled) { color: var(--control-ink); background: var(--text); }
.control-primary:disabled { background: var(--surface-raised); color: var(--text-faint); opacity: 1; }

.control-danger { color: var(--bad); border-color: var(--bad); }
.control-danger:hover:not(:disabled) { color: var(--bad); background: rgba(226, 112, 95, 0.12); }

/* A control that reads as part of a sentence rather than as a box. */
.control-link {
  border-color: transparent;
  padding: 0;
  text-decoration: underline;
  text-underline-offset: 3px;
}
.control-link:hover:not(:disabled) { border-color: transparent; }
</style>
