// The theme in force, applied to <html> and kept in step with the system.

import { ref } from "vue";
import { KEY, choiceOf, resolve, type Choice, type Look } from "../lib/theme";

const dark = window.matchMedia("(prefers-color-scheme: dark)");
const choice = ref<Choice>(choiceOf(localStorage.getItem(KEY)));

/** Put a look on the page, and on the window's own title bar. */
export function wear(look: Look) {
  const html = document.documentElement;
  html.dataset.theme = look.theme;
  html.dataset.tone = look.tone;
  // The native title bar and traffic lights; absent in a plain browser.
  void import("@tauri-apps/api/window")
    .then(({ getCurrentWindow }) => getCurrentWindow().setTheme(look.tone))
    .catch(() => {});
}

const apply = () => wear(resolve(choice.value, dark.matches));

/** Wear the stored theme, and follow the system while it is "system".
 *  Called once, before the app mounts. */
export function startTheme() {
  apply();
  dark.addEventListener("change", apply);
}

export function useTheme() {
  return {
    choice,
    choose(next: Choice) {
      choice.value = next;
      localStorage.setItem(KEY, next);
      apply();
    },
  };
}
