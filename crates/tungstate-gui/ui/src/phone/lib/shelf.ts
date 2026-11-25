// The last listing of each place, so coming back up to a folder draws it at
// once while a fresh one is fetched behind it, as ES does.
//
// Bounded by rows rather than places: one Downloads category can hold more
// rows than forty ordinary folders, and the phone's web view is short on
// memory. The least recently looked at goes first.

export interface Shelf<T> {
  get(key: string): T[] | undefined;
  put(key: string, rows: T[]): void;
  /** Rows held across every place, for the test. */
  held(): number;
}

export function makeShelf<T>(most = 50_000): Shelf<T> {
  // A Map iterates in insertion order, so re-inserting on every touch keeps
  // the stalest key first and eviction is one step from the front.
  const kept = new Map<string, T[]>();
  let total = 0;
  const drop = (key: string) => {
    total -= kept.get(key)?.length ?? 0;
    kept.delete(key);
  };
  return {
    get(key) {
      const rows = kept.get(key);
      if (rows) {
        kept.delete(key);
        kept.set(key, rows);
      }
      return rows;
    },
    put(key, rows) {
      drop(key);
      // Too big to keep at all: keeping it would push out everything else.
      if (rows.length > most) return;
      kept.set(key, rows);
      total += rows.length;
      for (const stalest of kept.keys()) {
        if (total <= most) break;
        drop(stalest);
      }
    },
    held: () => total,
  };
}
