// What happened lately, in sentences rather than one row per file.
//
// The journal keeps one op per file, so a tidy of forty files is forty rows
// that all say "rename" at the same minute. Home says it once: "Renamed 40
// files in Downloads". Ops run together into one line when they are the same
// kind, ended the same way, belong to the same transfer and happened in the
// same folder within a few minutes of each other.

import type { Notice, Op } from "../engine/types";
import { grouped, noticed, files, type FileCount } from "./counts";
import { plural } from "./format";
import { toneOfStatus, type Tone } from "./tone";

export interface Line {
  /** The newest moment in the line, in milliseconds. */
  at: number;
  tone: Tone;
  text: string;
  /** Set when the watcher did it rather than a person. */
  by?: "watcher";
}

/** Ops further apart than this start a new line even when alike. */
const APART_MS = 10 * 60_000;
/** How long before a watcher notice its own ops can have started. */
const WATCHER_MS = 2 * 60_000;

const DONE: Record<string, string> = {
  copy: "Copied", move: "Moved", rename: "Renamed", remove: "Removed", mkdir: "Made", rmdir: "Removed",
};
const DO: Record<string, string> = {
  copy: "copy", move: "move", rename: "rename", remove: "remove", mkdir: "make", rmdir: "remove",
};
const FOLDERS = new Set(["mkdir", "rmdir"]);

const parent = (path: string | null) => {
  const p = (path ?? "").replace(/\/+$/, "");
  const cut = Math.max(p.lastIndexOf("/"), p.lastIndexOf(":"));
  return cut > 0 ? p.slice(0, cut) : p;
};
export const leaf = (path: string | null) => (path ?? "").replace(/[/:]+$/, "").split(/[/:]/).pop() ?? "";

/** Where a line happened: a transfer by where it went, the rest by folder. */
const where = (op: Op) => (op.link ? parent(op.destination) : parent(op.source ?? op.destination));

function sentence(first: Op, count: FileCount | number): string {
  const kind = first.kind;
  const tone = toneOfStatus(first.status);
  const one = count === 1;
  // A folder op counts folders; every other kind moves one file per op.
  const what = FOLDERS.has(kind)
    ? one ? `the folder ${leaf(first.destination ?? first.source)}` : plural(count, "folder")
    : one ? leaf(first.destination ?? first.source) : files(count as FileCount);
  const place = first.link ? `to ${leaf(where(first))}` : `in ${leaf(where(first))}`;
  if (tone === "bad") return `Could not ${DO[kind] ?? kind} ${what} ${place}`;
  if (tone === "hold") return `Stopped part-way: ${DO[kind] ?? kind} ${what} ${place}`;
  if (first.status === "skipped") return `Left ${what} ${place} alone`;
  return `${DONE[kind] ?? kind} ${what} ${place}`;
}

/**
 * The newest `limit` lines from the journal's ops and the watcher's notices.
 * `roots` names each watched folder's path, so the ops a watcher notice
 * already speaks for are not said a second time.
 */
export function lately(ops: Op[], notices: Notice[], roots: Map<string, string>, limit: number): Line[] {
  const filed = notices.filter((n) => n.kind === "tidied");
  const saidByWatcher = (op: Op) =>
    filed.some((n) => {
      const root = roots.get(n.folder);
      return !!root && (op.source ?? "").startsWith(`${root}/`) && op.started_at <= n.at && n.at - op.started_at <= WATCHER_MS;
    });

  const lines: Line[] = filed.map((n) => ({ at: n.at, tone: "ok", text: `Tidied ${files(noticed(n))} in ${n.folder}`, by: "watcher" }));

  const sorted = ops.filter((op) => !saidByWatcher(op)).sort((a, b) => b.started_at - a.started_at);
  let run: Op[] = [];
  const close = () => {
    if (!run.length) return;
    const first = run[0]!;
    lines.push({ at: first.started_at, tone: toneOfStatus(first.status), text: sentence(first, FOLDERS.has(first.kind) ? run.length : grouped(run)) });
    run = [];
  };
  for (const op of sorted) {
    const head = run[0];
    const alike = head && head.kind === op.kind && head.status === op.status && head.link === op.link
      && where(head) === where(op) && head.started_at - op.started_at <= APART_MS;
    if (!alike) close();
    run.push(op);
  }
  close();

  return lines.sort((a, b) => b.at - a.at).slice(0, limit);
}
