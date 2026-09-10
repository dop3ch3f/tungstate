// Every call the window can make into the engine, in one place, typed once.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface Link {
  name: string;
  source: string;
  destination: string;
  source_policy: string;
  verify: string;
  order: string;
  on_conflict: string;
  cooldown_secs: number;
}

export interface Op {
  id: number;
  status: string;
  kind: string;
  source: string | null;
  destination: string | null;
  size: number | null;
  hash: string | null;
  link: string | null;
  note: string | null;
  started_at: number;
}

export interface Failure { path: string; reason: string }

export interface Summary {
  transferred: number;
  already_present: number;
  skipped: number;
  quarantined: number;
  failed: number;
  bytes: number;
  recovered: number;
  pruned: number;
  cancelled: boolean;
  failures: Failure[];
}

export interface Started { path: string; size: number }
export interface Finished { path: string; outcome: string; detail: string | null }
export interface ConflictAsk { path: string; incoming_size: number; existing_size: number }

export const api = {
  links: () => invoke<Link[]>("list_links"),
  createLink: (form: Omit<Link, never>) => invoke<void>("create_link", { form }),
  run: (name: string) => invoke<void>("run_link", { name }),
  cancel: () => invoke<void>("cancel_run"),
  resolve: (action: string, applyToAll: boolean) =>
    invoke<void>("resolve_conflict", { action, applyToAll }),
  history: (path: string) => invoke<Op[]>("history", { path }),
  whereis: (target: string) => invoke<Op[]>("whereis", { target }),
  recent: () => invoke<Op[]>("recent"),
  quarantined: (link: string) => invoke<string[]>("quarantined", { link }),
};

export const on = {
  started: (f: (e: Started) => void) => listen<Started>("transfer://started", (e) => f(e.payload)),
  finished: (f: (e: Finished) => void) => listen<Finished>("transfer://finished", (e) => f(e.payload)),
  conflict: (f: (e: ConflictAsk) => void) => listen<ConflictAsk>("transfer://conflict", (e) => f(e.payload)),
  done: (f: (e: Summary) => void) => listen<Summary>("transfer://done", (e) => f(e.payload)),
  failed: (f: (e: string) => void) => listen<string>("transfer://error", (e) => f(e.payload)),
};

/** Sizes read as mass here: this is a tool about reclaiming weight from a disk. */
export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = n / 1024;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) { value /= 1024; i++; }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[i]}`;
}

export function when(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });
}
