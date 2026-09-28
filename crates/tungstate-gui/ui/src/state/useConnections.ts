// Every connection, how each was last checked, and what a check just found.
//
// Module-level, like the other stores: the Connections section, the place
// picker and the Transfer panes all read one list, so adding a connection in
// one shows up in the others without anyone reloading.

import { ref, shallowRef } from "vue";
import { connections } from "../engine/commands";
import type { Connection, Probe } from "../engine/types";
import { useBusy } from "./useBusy";

const all = shallowRef<Connection[]>([]);
/** What the last check this session found, for its list of names. The
 *  outcome itself is on the connection, remembered by the engine. */
const found = shallowRef<Record<string, Probe>>({});
const problem = ref<string | null>(null);
const loaded = ref(false);
/** A place the Transfer panes should open next, set from another section. */
const browseTo = ref<string | null>(null);

const doing = useBusy();

async function load() {
  try {
    all.value = await connections.list();
    problem.value = null;
  } catch (e) {
    problem.value = String(e);
  } finally {
    loaded.value = true;
  }
}

/** Check one, without reloading the list, for `checkAll` to share one reload. */
async function probe(name: string) {
  try {
    const got = await connections.test(name);
    found.value = { ...found.value, [name]: got };
  } catch {
    // The failure is recorded with the connection and shown on its row.
    const { [name]: _gone, ...rest } = found.value;
    found.value = rest;
  }
}

const check = (name: string) =>
  doing.run(`check:${name}`, async () => {
    await probe(name);
    await load();
  });

/** Check every connection at once, and reload once when all have answered. */
const checkAll = () =>
  doing.run("check-all", async () => {
    await Promise.all(all.value.map((c) => doing.run(`check:${c.name}`, () => probe(c.name))));
    await load();
  });

export function useConnections() {
  return {
    all, found, problem, loaded, browseTo,
    load, check, checkAll,
    checking: (name: string) => doing.busy(`check:${name}`) || doing.busy("check-all"),
    checkingAll: () => doing.busy("check-all"),
  };
}
