// One question at a time, asked from anywhere, answered by an id.
//
// This replaces twelve hand-rolled `.veil` / `.modal` pairs. None of them
// trapped focus, none handled Escape, and all twelve repeated the same markup.
//
// The behaviour that must survive the consolidation is the checkbox reset.
// `App.vue:298-299` sets `applyAll` back to false after every conflict answer.
// A shared host that kept its checkbox between calls would silently apply the
// first answer to every later conflict, on somebody's files. `ask()` clears it
// on every call, and that is not an implementation detail.

import { ref, shallowRef } from "vue";

export interface Choice {
  id: string;
  label: string;
  look?: "plain" | "primary" | "danger";
}

export interface Question {
  title: string;
  /** The sentence under the title. What this does, in plain words. */
  why?: string;
  /** Lines of detail: counts, paths, whatever the person needs to decide. */
  detail?: string[];
  /** An optional tick, e.g. "do this for the rest of them". Always starts off. */
  checkbox?: string;
  choices: Choice[];
}

export interface Answer {
  /** The chosen id, or null when dismissed. */
  id: string | null;
  checked: boolean;
}

const open = shallowRef<Question | null>(null);
const checked = ref(false);
let settle: ((a: Answer) => void) | null = null;

/** Ask, and wait. Dismissing resolves with `{ id: null }` rather than rejecting:
 *  a person closing a dialog has answered, and callers should not need a
 *  `try`. */
export function ask(question: Question): Promise<Answer> {
  checked.value = false; // never inherited from the last question
  open.value = question;
  return new Promise<Answer>((resolve) => {
    settle = resolve;
  });
}

export function answer(id: string | null) {
  const done = settle;
  const was = checked.value;
  open.value = null;
  settle = null;
  done?.({ id, checked: was });
}

export const dialog = { open, checked };
