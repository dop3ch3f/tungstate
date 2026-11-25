// The three ways a file window lists, as ES File Explorer has them.

/** Small icons; a bigger preview with details; a grid of previews. */
export type View = "list" | "detailed" | "tiles";

/** The remembered view. "grid" is what the two-view build stored; it is the
 *  tiles now, so nobody's choice is lost. A fresh install shows previews. */
export function storedView(raw: string | null): View {
  if (raw === "grid") return "tiles";
  return raw === "list" || raw === "detailed" || raw === "tiles" ? raw : "detailed";
}
