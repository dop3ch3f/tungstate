// What the Moving screen counts, from the engine's per-file ledger. Pure, so
// the rules are tested apart from the screen.

/** A ledger row as `useTransfer` keeps it; `state` is the engine's word. */
export interface LedgerRow {
  path: string;
  size: number;
  state: string;
  done: number;
}

/** Reached the far side and was compared there: the only proof a Move
 *  goes by before it removes an original. */
const CHECKED = new Set(["transferred", "already-present"]);
const OPEN = new Set(["waiting", "live", "checking"]);

export interface Tally {
  sent: number;
  checked: number;
  failed: number;
  count: number;
  bytes: number;
  total: number;
}

export function tally(rows: LedgerRow[]): Tally {
  const t: Tally = { sent: 0, checked: 0, failed: 0, count: rows.length, bytes: 0, total: 0 };
  for (const r of rows) {
    t.total += r.size;
    if (r.state === "waiting") continue;
    t.bytes += r.state === "live" || r.state === "checking" ? r.done : r.size;
    if (r.state !== "live") t.sent++;
    if (CHECKED.has(r.state)) t.checked++;
    else if (r.state === "failed") t.failed++;
  }
  return t;
}

/**
 * Which of the chosen locations still have something not checked, after a
 * run that stopped part-way. A chosen folder counts as one: if anything in
 * it is not done, the folder goes again, and only what is left in it moves.
 * Rows are named relative to the folder they were sent from, so a chosen
 * item is matched by its own name or as the folder its rows sit under.
 */
export function unfinished(chosen: string[], rows: LedgerRow[], nameOf: (p: string) => string): string[] {
  const open = rows.filter((r) => !CHECKED.has(r.state));
  return chosen.filter((p) => {
    const name = nameOf(p);
    const mine = rows.filter((r) => r.path === name || r.path.startsWith(`${name}/`));
    // Never reached by the run at all: still to do.
    if (!mine.length) return true;
    return open.some((r) => r.path === name || r.path.startsWith(`${name}/`));
  });
}
