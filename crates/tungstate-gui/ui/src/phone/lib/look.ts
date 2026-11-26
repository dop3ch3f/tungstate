// How a file or folder looks in a phone list: its pixel mark and the colour
// of its kind, the desktop's kind colours. One place, so every window and the
// Connections tab draw a file the same way.

import type { Category } from "../../engine/types";
import type { Art } from "../../lib/pixels";
import { kind } from "../../lib/format";
import { connectionOf, topOf, type Top } from "./paths";

const TINT: Record<string, string> = {
  Folder: "var(--raw-kind-folder)",
  Video: "var(--raw-kind-video)",
  Image: "var(--raw-kind-image)",
  "Raw image": "var(--raw-kind-image)",
  Audio: "var(--raw-kind-audio)",
  Archive: "var(--raw-kind-archive)",
  Document: "var(--raw-kind-doc)",
  Sheet: "var(--raw-kind-doc)",
};

const ART: Record<string, Art> = {
  Folder: "folder",
  Video: "video",
  Image: "photo",
  "Raw image": "photo",
  Audio: "music",
  Document: "doc",
  Sheet: "doc",
};

type Named = { is_dir: boolean; name: string };

export const tintOf = (e: Named): string => TINT[kind(e)] ?? "#a5a79d";
export const artOf = (e: Named): Art => ART[kind(e)] ?? "file";

/** The category tiles, in the approved order, with their marks and colours. */
export const CATEGORIES: { id: Category; label: string; art: Art; tint: string }[] = [
  { id: "images", label: "Images", art: "photo", tint: "var(--raw-kind-image)" },
  { id: "videos", label: "Videos", art: "video", tint: "var(--raw-kind-video)" },
  { id: "audio", label: "Music", art: "music", tint: "var(--raw-kind-audio)" },
  { id: "documents", label: "Documents", art: "doc", tint: "var(--raw-kind-doc)" },
  { id: "downloads", label: "Downloads", art: "download", tint: "var(--raw-kind-folder)" },
  { id: "apps", label: "Apps", art: "apps", tint: "var(--raw-kind-archive)" },
];

/** A place's mark on its colour square: a tab, a storage row, a connection. */
export interface Look {
  art: Art;
  tint: string;
  /** White ink, on the darker colours. */
  light: boolean;
}

export const HOME_LOOK: Look = { art: "home", tint: "var(--tint-home)", light: false };
export const PHONE_LOOK: Look = { art: "phone", tint: "var(--tint-home)", light: false };
export const CARD_LOOK: Look = { art: "sd", tint: "#a5a79d", light: false };

/** Each kind of connection in a desktop section colour of its own, so two
 *  tabs showing a "Photos" on different places are told apart at a glance. */
const SCHEME_TINT: Record<string, string> = {
  smb: "var(--tint-connections)",
  sftp: "var(--tint-drain)",
  webdav: "var(--tint-folder)",
  ftp: "var(--tint-sync)",
  ftps: "var(--tint-sync)",
  s3: "var(--tint-dupes)",
};
export const connectionLook = (scheme: string | undefined): Look =>
  ({ art: "nas", tint: (scheme && SCHEME_TINT[scheme]) || "var(--tint-connections)", light: true });

/** The mark of whatever a folder is on: its connection's kind, a card, or
 *  this phone. */
export function folderLook(path: string, volumes: (Top & { removable: boolean })[], schemeOf: (connection: string) => string | undefined): Look {
  const conn = connectionOf(path);
  if (conn !== null) return connectionLook(schemeOf(conn));
  const top = topOf(path, volumes);
  return top && "removable" in top && top.removable ? CARD_LOOK : PHONE_LOOK;
}

export const CATEGORY_LABEL = Object.fromEntries(CATEGORIES.map((c) => [c.id, c.label])) as Record<Category, string>;

const PICTURE = /\.(jpe?g|png|gif|webp|heic|heif|bmp|avif|tiff?)$/i;
const MOVING = /\.(mp4|mov|m4v|mkv|webm|3gp|avi)$/i;
const SOUND = /\.(mp3|m4a|flac|ogg|opus|wav|aac)$/i;

/** A photo the viewer can show. */
export const isPicture = (name: string): boolean => PICTURE.test(name);

/**
 * Whether asking for a preview can come back with one. On this phone, the
 * system draws photos, video frames, song covers and app icons; on a
 * connection only photos are worth the network read.
 */
export function hasPicture(e: { name: string; is_dir: boolean; path: string }): boolean {
  if (e.is_dir) return false;
  // `name:path` is a connection; `content://` is this phone's photo library.
  if (/^[^/:]+:(?!\/\/)/.test(e.path)) return PICTURE.test(e.name);
  return PICTURE.test(e.name) || MOVING.test(e.name) || SOUND.test(e.name) || /\.apk$/i.test(e.name);
}
