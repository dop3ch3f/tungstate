// Which rows of a long list to draw.
//
// A browser draws a few thousand rows slowly and ten thousand not at all, and
// a transfer of every photograph on a laptop is ten thousand rows.

/** The most rows drawn at once. Filtering reaches the rest. */
export const DRAWN = 200;

/** Settled rows kept above the first busy one, so the reader sees what just
 *  finished as well as what is happening. */
const CONTEXT = 20;

/**
 * At most `cap` of `rows`, and where they start. When `follow` is set the
 * slice starts just above the first busy row, which on a running transfer is
 * where the work is; otherwise it is the top of the list.
 */
export function inView<T>(rows: T[], busy: (row: T) => boolean, follow: boolean, cap = DRAWN): { from: number; rows: T[] } {
  if (rows.length <= cap) return { from: 0, rows };
  let from = 0;
  if (follow) {
    const at = rows.findIndex(busy);
    if (at > 0) from = Math.min(Math.max(0, at - CONTEXT), rows.length - cap);
  }
  return { from, rows: rows.slice(from, from + cap) };
}
