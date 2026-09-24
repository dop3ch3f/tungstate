// Turning engine values into words. Pure, and importing nothing, so any screen
// can reach for it without dragging Tauri in behind it.

/** Sizes read as mass here: this is a tool about reclaiming weight from a disk. */
export function bytes(n: number): string {
  if (n < 1000) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = n / 1024;
  let i = 0;
  // Step up at 1000, not 1024, so a size never shows four digits: 1,000 MiB
  // reads "1.0 GB" rather than "1000 MB".
  while (value >= 1000 && i < units.length - 1) {
    value /= 1024;
    i += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[i]}`;
}

/** `1 group`, `3 groups`: for counts that are not files. Files go through
 *  `counts.ts`, which checks where the number came from. */
export const plural = (n: number, one: string, many = `${one}s`): string => `${n} ${n === 1 ? one : many}`;

/**
 * Shorten a path from the left, keeping the part that identifies it.
 *
 * `direction: rtl` truncates from the right end but reorders the slashes, so
 * `/Users/me/Movies` renders as `Users/me/Movies/`. Doing it in script keeps
 * the string honest.
 */
export function shortPath(path: string, keep = 3): string {
  const parts = path.split("/").filter(Boolean);
  if (parts.length <= keep) return path;
  return "…/" + parts.slice(-keep).join("/");
}

/** A wait in words. Slice 7b shipped "the longest has 3351s to wait" on the
 *  screen whose whole premise is plain words. */
export function duration(seconds: number): string {
  if (seconds < 60) return `${Math.round(seconds)} seconds`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? "" : "s"}`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"}`;
  const days = Math.round(hours / 24);
  return `${days} day${days === 1 ? "" : "s"}`;
}

export function when(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** A human name for what a file is. */
export function kind(entry: { is_dir: boolean; name: string }): string {
  if (entry.is_dir) return "Folder";
  if (!entry.name.includes(".")) return "File";
  const ext = entry.name.split(".").pop()?.toLowerCase() ?? "";
  const groups: Record<string, string[]> = {
    Video: ["mp4", "mov", "mkv", "avi", "m4v", "webm", "mpg", "mpeg", "wmv", "flv", "mts", "m2ts"],
    Image: ["jpg", "jpeg", "png", "heic", "heif", "gif", "tiff", "tif", "webp", "bmp", "svg"],
    "Raw image": ["raw", "cr2", "cr3", "nef", "arw", "dng", "orf", "raf"],
    Audio: ["mp3", "wav", "flac", "aac", "m4a", "aiff", "ogg", "opus"],
    Archive: ["zip", "tar", "gz", "bz2", "xz", "7z", "rar", "dmg", "iso"],
    Document: ["pdf", "doc", "docx", "pages", "txt", "md", "rtf", "odt"],
    Sheet: ["xls", "xlsx", "numbers", "csv", "tsv"],
    Project: ["fcpbundle", "prproj", "aep", "logicx", "als", "ptx"],
  };
  for (const [label, exts] of Object.entries(groups)) {
    if (exts.includes(ext)) return label;
  }
  return ext.toUpperCase();
}

