// File servers on this Wi-Fi, for Files home and Connections to offer.
// Module-level, so both screens show one list and one look at a time.

import { computed, ref, shallowRef } from "vue";
import { listen } from "@tauri-apps/api/event";
import { network } from "../../engine/commands";
import type { FoundServer } from "../../engine/types";
import { useConnections } from "../../state/useConnections";
import { unsaved, upsert } from "../lib/discovery";

const found = shallowRef<FoundServer[]>([]);
const looking = ref(false);
/** Whether a look has finished at least once, so "nothing found" can be said. */
const looked = ref(false);
/** The server the person tapped Add on, for Connections to open the form with. */
const adding = shallowRef<FoundServer | null>(null);

let listening: Promise<unknown> | null = null;

async function look() {
  if (looking.value) return;
  looking.value = true;
  // Rows arrive as they are seen, so the list fills in during the look
  // rather than after it.
  listening ??= listen<FoundServer>("discover://found", (e) => (found.value = upsert(found.value, e.payload))).catch(() => null);
  await listening;
  try {
    const all = await network.discover();
    if (all) for (const one of all) found.value = upsert(found.value, one);
  } catch {
    // Not on a network the phone may look at; the list stays as it is.
  } finally {
    looking.value = false;
    looked.value = true;
  }
}

export function useDiscovery() {
  const conns = useConnections();
  /** What is here and not saved yet. */
  const fresh = computed(() => unsaved(found.value, conns.all.value));
  return { found, fresh, looking, looked, adding, look };
}
