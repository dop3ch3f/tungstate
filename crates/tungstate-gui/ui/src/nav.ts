// Where in the window you are. The router this application does not have.
//
// A module-level ref and a function, for the same reason the state modules are
// module-level: importing this twice gives the same value, and a desktop
// window with one frame does not need anything larger. There is no history and
// no deep link, because a Tauri window has no address bar to put one in.

import { ref } from "vue";

export type View = "home" | "folder" | "drain" | "history";

const view = ref<View>("home");

export function useNav() {
  return { view, go: (to: View) => (view.value = to) };
}
