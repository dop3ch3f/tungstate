// How a file or folder looks in a phone list: its pixel mark and the colour
// of its kind, the desktop's kind colours. One place, so every window and the
// Connections tab draw a file the same way.

import type { Category } from "../../engine/types";
import type { Art } from "../../lib/pixels";
import { kind } from "../../lib/format";

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
