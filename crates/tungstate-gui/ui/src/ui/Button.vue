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
  background: var(--panel);
  border: var(--bw) solid var(--edge);
  border-radius: var(--radius);
  min-height: 30px;
  white-space: nowrap;
  padding: 0 var(--s4);
  cursor: pointer;
  box-shadow: var(--lift);
  transition: background var(--quick) var(--ease), border-color var(--quick) var(--ease);
}
.control:hover:not(:disabled) { color: var(--text); background: var(--surface-raised); }
.control:disabled { color: var(--disabled); border-color: var(--edge); opacity: 0.6; box-shadow: none; cursor: default; }

.control-plain { /* the default, already drawn above */ }

.control-primary {
  font-size: var(--small);
  font-weight: 600;
  color: var(--control-ink);
  background: var(--control);
  border-color: transparent;
  padding: 0 var(--s4);
}
.control-primary:hover:not(:disabled) { color: var(--control-ink); background: var(--control); filter: brightness(1.06); }
.control-primary:disabled { background: var(--surface-raised); color: var(--disabled); border-color: var(--edge); box-shadow: none; opacity: 1; }

.control-danger { color: var(--bad); border-color: var(--bad); }
.control-danger:hover:not(:disabled) { color: var(--bad); background: var(--surface-raised); }

/* A control that reads as part of a sentence rather than as a box. */
.control-link {
  background: none;
  border-color: transparent;
  border-radius: 0;
  min-height: 0;
  box-shadow: none;
  padding: 0;
  text-decoration: underline;
  text-underline-offset: 3px;
}
.control-link:hover:not(:disabled) { background: none; }
</style>
