// Locations as the phone's file windows hold them: the engine's composite
// strings, `/storage/emulated/0/DCIM` or `Area51:Photos/2026`. Pure, so the
// rules about where a path's folder and name are can be tested alone.
//
// A remote is `name:path` with `/` between folders whatever the phone uses;
// a local path starts with `/`. Same reading as the engine's `ends.rs`.

/** The connection a location is on, or null for this phone. */
export function connectionOf(path: string): string | null {
  const m = /^([^/:]+):/.exec(path);
  return m ? m[1]! : null;
}

/** The folder a location is in, or null at the top of a place. */
export function parentOf(path: string): string | null {
  const conn = connectionOf(path);
  if (conn !== null) {
    const rest = path.slice(conn.length + 1).replace(/\/+$/, "");
    if (!rest) return null;
    const cut = rest.lastIndexOf("/");
    return `${conn}:${cut < 0 ? "" : rest.slice(0, cut)}`;
  }
  const trimmed = path.replace(/\/+$/, "");
  if (!trimmed) return null;
  const cut = trimmed.lastIndexOf("/");
  return cut <= 0 ? "/" : trimmed.slice(0, cut);
}

/** The last part of a location: a file's or folder's own name. */
export function nameOf(path: string): string {
  const conn = connectionOf(path);
  const rest = (conn !== null ? path.slice(conn.length + 1) : path).replace(/\/+$/, "");
  return rest.slice(rest.lastIndexOf("/") + 1);
}

/** A child of a folder, joined the way the engine's `join_display` does. */
export function childOf(folder: string, name: string): string {
  if (connectionOf(folder) !== null && folder.endsWith(":")) return `${folder}${name}`;
  return `${folder.replace(/\/+$/, "")}/${name}`;
}

/** Where a place starts and what to call it: a volume, or a connection. */
export interface Top {
  label: string;
  path: string;
}

/** The deepest top holding `path`, so `/storage/emulated/0/DCIM` is under
 *  Internal storage and not under `/`. A connection is its own top. */
export function topOf(path: string, tops: Top[]): Top | null {
  const conn = connectionOf(path);
  if (conn !== null) return { label: conn, path: `${conn}:` };
  let best: Top | null = null;
  for (const t of tops) {
    const root = t.path.replace(/\/+$/, "");
    if ((path === root || path.startsWith(`${root}/`)) && (!best || root.length > best.path.length)) best = t;
  }
  return best;
}

/**
 * The path chips above a folder: its top, then each folder down to the one
 * above it. The folder itself is the title, so it is not a chip.
 */
export function crumbs(path: string, tops: Top[]): Top[] {
  const top = topOf(path, tops);
  if (!top) return [];
  const chain: Top[] = [];
  let at = parentOf(path);
  const root = top.path.replace(/\/+$/, "") || top.path;
  while (at !== null && at.length > root.length) {
    chain.unshift({ label: nameOf(at), path: at });
    at = parentOf(at);
  }
  return path === root || path === top.path ? [] : [top, ...chain];
}

/** One leg per source folder: the engine moves names out of one folder at a
 *  time, and a selection from a category spans many. */
export function legsFor(paths: string[], destination: string): { source: string; destination: string; names: string[] }[] {
  const by = new Map<string, string[]>();
  for (const p of paths) {
    const folder = parentOf(p);
    if (folder === null) continue;
    const names = by.get(folder);
    if (names) names.push(nameOf(p));
    else by.set(folder, [nameOf(p)]);
  }
  return [...by].map(([source, names]) => ({ source, destination, names }));
}

/** A location in words, as the screens show it: `Area51 › Photos › 2026`,
 *  `Internal storage › DCIM`. Never the raw string. */
export function placeName(path: string, tops: Top[]): string {
  const top = topOf(path, tops);
  if (!top) return path;
  const isTop = path.replace(/\/+$/, "") === top.path.replace(/\/+$/, "") || path === top.path;
  return [...crumbs(path, tops).map((c) => c.label), isTop ? top.label : nameOf(path)].join(" › ");
}
