// Search, sort and filters for any list, so every table behaves the same way.
//
// The rows stay the caller's; this only decides which of them show and in what
// order. It is pure over a `Ref`, so a table re-derives when its rows change
// and never holds a copy that can drift from them.

import { computed, ref, type Ref } from "vue";

/** A column that can be sorted by. */
export interface Column<T> {
  key: string;
  value: (row: T) => string | number | null;
}

/** A way to narrow rows by one property, shown as a row of chips. */
export interface Facet<T> {
  key: string;
  label: string;
  of: (row: T) => string;
}

export interface TableOptions<T> {
  columns: Column<T>[];
  /** Everything a search should find a row by, as one string. */
  text: (row: T) => string;
  facets?: Facet<T>[];
  sort?: { key: string; dir: 1 | -1 };
  /** Kept above everything else whatever the sort, e.g. folders above files. */
  pinned?: (row: T) => boolean;
}

export type Table<T> = ReturnType<typeof useTable<T>>;

// One collator, built once: `localeCompare` builds a new one per call, which
// is most of the cost of sorting a few thousand names.
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

export function useTable<T>(rows: Ref<T[]>, options: TableOptions<T>) {
  const query = ref("");
  const sort = ref(options.sort ?? { key: options.columns[0]?.key ?? "", dir: 1 as 1 | -1 });
  /** Chosen value per facet; a missing key means "all". */
  const chosen = ref<Record<string, string>>({});

  // Lowercased once per row change, not once per keystroke per row.
  const haystacks = computed(() => rows.value.map((row) => options.text(row).toLowerCase()));

  const matching = computed(() => {
    const needle = query.value.trim().toLowerCase();
    const picks = Object.entries(chosen.value);
    const facetOf = new Map((options.facets ?? []).map((f) => [f.key, f.of]));
    return rows.value.filter(
      (row, index) =>
        (!needle || haystacks.value[index]!.includes(needle)) &&
        picks.every(([key, value]) => facetOf.get(key)?.(row) === value),
    );
  });

  const shown = computed(() => {
    const column = options.columns.find((c) => c.key === sort.value.key);
    const { dir } = sort.value;
    const pinned = options.pinned;
    return [...matching.value].sort((a, b) => {
      if (pinned) {
        const pa = pinned(a), pb = pinned(b);
        if (pa !== pb) return pa ? -1 : 1;
      }
      if (!column) return 0;
      const va = column.value(a), vb = column.value(b);
      // Empty values sink to the bottom in either direction.
      if (va === null || va === "") return vb === null || vb === "" ? 0 : 1;
      if (vb === null || vb === "") return -1;
      const r = typeof va === "number" && typeof vb === "number" ? va - vb : collator.compare(String(va), String(vb));
      return r * dir;
    });
  });

  /** Each facet's values with how many of all the rows carry them. Counted
   *  over every row, not the narrowed ones, so choosing a chip never makes
   *  the others vanish or read zero. */
  const facetValues = computed(() =>
    (options.facets ?? []).map((facet) => {
      const counts = new Map<string, number>();
      for (const row of rows.value) {
        const value = facet.of(row);
        counts.set(value, (counts.get(value) ?? 0) + 1);
      }
      return {
        key: facet.key,
        label: facet.label,
        values: [...counts.entries()]
          .sort((a, b) => b[1] - a[1] || collator.compare(a[0], b[0]))
          .map(([value, count]) => ({ value, count })),
      };
    }),
  );

  /** Click a header: the same column flips direction, a new one starts ascending
   *  for words and descending for numbers, which is what anyone wants first. */
  function by(key: string, numeric = false) {
    sort.value =
      sort.value.key === key
        ? { key, dir: sort.value.dir === 1 ? -1 : 1 }
        : { key, dir: numeric ? -1 : 1 };
  }

  function pick(facet: string, value: string | null) {
    const next = { ...chosen.value };
    if (value === null || next[facet] === value) delete next[facet];
    else next[facet] = value;
    chosen.value = next;
  }

  const narrowed = computed(() => query.value.trim() !== "" || Object.keys(chosen.value).length > 0);

  function clear() {
    query.value = "";
    chosen.value = {};
  }

  return { query, sort, chosen, shown, facetValues, by, pick, narrowed, clear, total: computed(() => rows.value.length) };
}
