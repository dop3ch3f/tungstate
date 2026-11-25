// What Duplicates and Organise need to say about a folder before anything
// moves. Pure, so the grouping and the choices can be tested alone.

/** Where a preview's moves land, one row per destination folder, biggest
 *  first: "2026 › Documents › pdf", 188. */
export function destinations(moves: { to: string }[]): { folder: string; files: number }[] {
  const by = new Map<string, number>();
  for (const m of moves) {
    const cut = m.to.lastIndexOf("/");
    const folder = cut < 0 ? "" : m.to.slice(0, cut);
    by.set(folder, (by.get(folder) ?? 0) + 1);
  }
  return [...by]
    .map(([folder, files]) => ({ folder: folder ? folder.split("/").join(" › ") : "The folder itself", files }))
    .sort((a, b) => b.files - a.files || a.folder.localeCompare(b.folder));
}

/** The starting layout that suits a folder by its name; `by-type` otherwise. */
export function layoutFor(folder: string): string {
  const name = (folder.replace(/\/+$/, "").split("/").pop() ?? "").toLowerCase();
  if (name === "download" || name === "downloads") return "downloads";
  if (name === "dcim" || name === "camera" || name === "pictures") return "photos";
  if (name === "documents") return "documents";
  if (name === "movies" || name === "music") return "media";
  return "by-type";
}

/** The folders worth offering under a phone's internal storage, with what
 *  each is called on screen. Only the ones that exist are shown. */
export function placesUnder(root: string, wanted: "dupes" | "organise"): { label: string; path: string }[] {
  const at = (rel: string) => `${root.replace(/\/+$/, "")}/${rel}`;
  return wanted === "dupes"
    ? [
        { label: "The whole phone", path: root },
        { label: "Camera", path: at("DCIM") },
        { label: "Pictures", path: at("Pictures") },
        { label: "Download", path: at("Download") },
        { label: "WhatsApp", path: at("Android/media/com.whatsapp/WhatsApp/Media") },
      ]
    : [
        { label: "Download", path: at("Download") },
        { label: "Documents", path: at("Documents") },
        { label: "Pictures", path: at("Pictures") },
        { label: "Music", path: at("Music") },
      ];
}
