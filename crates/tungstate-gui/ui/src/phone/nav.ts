// Which tab the phone app is on. Module-level, like the desktop's `nav.ts`,
// so any screen can send you to another tab ("2,140 duplicates ›").

import { ref } from "vue";

export type Tab = "home" | "free" | "dupes" | "organise" | "connections";

const tab = ref<Tab>("home");

export function usePhoneNav() {
  return { tab, go: (to: Tab) => (tab.value = to) };
}
