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

export interface Entry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: number | null;
}

export interface Listing {
  path: string;
  parent: string | null;
  entries: Entry[];
}

export interface Place { label: string; path: string }

export interface TransferRequest {
  source: string;
  destination: string;
  names: string[];
  source_policy: string;
  verify: string;
  on_conflict: string;
  save_as: string | null;
}

export const api = {
  browse: (path: string) => invoke<Listing>("browse", { path }),
  places: () => invoke<Place[]>("places"),
  lastPanes: () => invoke<{ left: string | null; right: string | null }>("last_panes"),
  rememberPanes: (panes: { left: string | null; right: string | null }) =>
    invoke<void>("remember_panes", { panes }),
  startTransfer: (request: TransferRequest) => invoke<string>("start_transfer", { request }),
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

/**
 * Shorten a path from the left, keeping the part that identifies it.
 *
 * CSS direction:rtl truncates paths from the correct end but reorders the
 * slashes, so `/Users/me/Movies` renders as `Users/me/Movies/`. Doing it in
 * script keeps the string honest.
 */
export function shortPath(path: string, keep = 3): string {
  const parts = path.split("/").filter(Boolean);
  if (parts.length <= keep) return path;
  return "…/" + parts.slice(-keep).join("/");
}

/** A quiet type mark rather than an icon set: no assets, no licensing, no weight. */
export function mark(entry: { is_dir: boolean; name: string }): string {
  if (entry.is_dir) return "▸";
  const ext = entry.name.split(".").pop()?.toLowerCase() ?? "";
  if (["mp4", "mov", "mkv", "avi", "m4v", "webm"].includes(ext)) return "▮";
  if (["jpg", "jpeg", "png", "heic", "gif", "tiff", "raw", "webp"].includes(ext)) return "◼";
  if (["mp3", "wav", "flac", "aac", "m4a"].includes(ext)) return "♪";
  if (["zip", "tar", "gz", "7z", "dmg", "iso"].includes(ext)) return "▦";
  return "·";
}

export function when(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });
}
