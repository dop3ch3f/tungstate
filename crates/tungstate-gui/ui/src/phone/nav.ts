// Which tab the phone app is on. Module-level, like the desktop's `nav.ts`,
// so any screen can send you to another tab ("+ Add" › Connections).

import { ref } from "vue";

export type Tab = "files" | "free" | "dupes" | "organise" | "connections";

const tab = ref<Tab>("files");

export function usePhoneNav() {
  return { tab, go: (to: Tab) => (tab.value = to) };
}
