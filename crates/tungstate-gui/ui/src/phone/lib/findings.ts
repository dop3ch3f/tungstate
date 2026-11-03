// Free up's findings as the list shows them: by year, newest first, each
// year with what is ticked in it. Pure, for the tests.

import type { PhotoAsset } from "../../engine/types";

export interface YearGroup {
  year: number;
  items: PhotoAsset[];
  bytes: number;
  ticked: number;
  tickedBytes: number;
}

/** `assets` come newest first from the engine, so years and the items in
 *  them keep that order without sorting again. */
export function byYear(assets: PhotoAsset[], unticked: Set<string>): YearGroup[] {
  const groups: YearGroup[] = [];
  let at: YearGroup | undefined;
  for (const a of assets) {
    if (at?.year !== a.year) {
      at = { year: a.year, items: [], bytes: 0, ticked: 0, tickedBytes: 0 };
      groups.push(at);
    }
    at.items.push(a);
    at.bytes += a.size;
    if (!unticked.has(a.id)) {
      at.ticked++;
      at.tickedBytes += a.size;
    }
  }
  return groups;
}

/** All, none or some of a year ticked, for its box: ✓, empty, or –. */
export const yearState = (g: YearGroup): "all" | "none" | "some" =>
  g.ticked === g.items.length ? "all" : g.ticked === 0 ? "none" : "some";
