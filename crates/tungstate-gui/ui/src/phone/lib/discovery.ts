// What a server found on the network means for the add-connection form.
// Pure, so the choices (which kind first, what it is called, whether it is
// already saved) can be tested without a network.

import type { FoundServer } from "../../engine/types";

export type FoundKind = FoundServer["offers"][number]["kind"];

/** Fastest and safest first: SMB is encrypted and quick on a LAN, SFTP is
 *  encrypted, WebDAV over HTTPS before plain, FTP last (it sends the
 *  password as it is). */
const RANK: FoundKind[] = ["smb", "sftp", "webdavs", "webdav", "ftp"];

const LABEL: Record<FoundKind, string> = { smb: "SMB", sftp: "SFTP", webdavs: "WebDAV", webdav: "WebDAV", ftp: "FTP" };

/** The kinds a server offers, best first. */
export function kindsOf(found: FoundServer): FoundKind[] {
  return [...found.offers].map((o) => o.kind).sort((a, b) => RANK.indexOf(a) - RANK.indexOf(b));
}

/** "SMB and FTP", "SFTP", "SMB, SFTP and FTP". */
export function offersLine(found: FoundServer): string {
  const labels = [...new Set(kindsOf(found).map((k) => LABEL[k]))];
  return labels.length <= 1 ? (labels[0] ?? "") : `${labels.slice(0, -1).join(", ")} and ${labels[labels.length - 1]}`;
}

/** The port to fill in: blank when it is the kind's usual one. */
export function portFor(found: FoundServer, kind: FoundKind): number | null {
  const usual: Record<FoundKind, number> = { smb: 445, ftp: 21, sftp: 22, webdav: 80, webdavs: 443 };
  const port = found.offers.find((o) => o.kind === kind)?.port ?? null;
  return port === null || port === usual[kind] ? null : port;
}

/** A connection name from what the server calls itself, fitted to the name
 *  rule (letters, numbers, - and _; two or more), and not one already taken. */
export function nameFor(found: FoundServer, taken: string[]): string {
  const fitted = (found.name ?? "")
    .trim()
    .replace(/\s+/g, "-")
    .replace(/[^A-Za-z0-9_-]/g, "")
    .slice(0, 40);
  const base = fitted.length >= 2 ? fitted : `NAS-${found.address.split(".").pop()}`;
  const lower = new Set(taken.map((t) => t.toLowerCase()));
  if (!lower.has(base.toLowerCase())) return base;
  for (let n = 2; ; n++) if (!lower.has(`${base}-${n}`.toLowerCase())) return `${base}-${n}`;
}

/** Servers not saved yet, by address or by `.local` name. */
export function unsaved(found: FoundServer[], saved: { host: string | null }[]): FoundServer[] {
  const hosts = new Set(saved.map((c) => (c.host ?? "").trim().toLowerCase().replace(/\.$/, "")).filter(Boolean));
  return found.filter((f) => {
    const names = [f.address, f.host, f.name && `${f.name}.local`].filter(Boolean).map((n) => n!.toLowerCase());
    return !names.some((n) => hosts.has(n));
  });
}

/** Folds one more sighting into the list, by address. */
export function upsert(list: FoundServer[], one: FoundServer): FoundServer[] {
  const at = list.findIndex((f) => f.address === one.address);
  if (at < 0) return [...list, one].sort((a, b) => Number(!a.name) - Number(!b.name) || a.address.localeCompare(b.address, undefined, { numeric: true }));
  const next = [...list];
  next[at] = one;
  return next;
}
