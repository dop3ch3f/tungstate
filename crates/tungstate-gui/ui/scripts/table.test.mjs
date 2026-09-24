// The table's search, sort and filters, which every long list leans on.
// Node's own runner with type stripping, so no test framework is installed.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ref } from "vue";
import { useTable } from "../src/lib/table.ts";

const files = () =>
  ref([
    { name: "b10.jpg", kind: "photo", size: 5, dir: false },
    { name: "b2.jpg", kind: "photo", size: 50, dir: false },
    { name: "notes.txt", kind: "text", size: null, dir: false },
    { name: "Archive", kind: "folder", size: 0, dir: true },
  ]);
const options = {
  columns: [
    { key: "name", value: (r) => r.name },
    { key: "size", value: (r) => r.size },
  ],
  text: (r) => r.name,
  facets: [{ key: "kind", label: "Kind", of: (r) => r.kind }],
};
const names = (t) => t.shown.value.map((r) => r.name);

test("names sort the way a person counts, not the way a byte does", () => {
  const t = useTable(files(), options);
  assert.deepEqual(names(t), ["Archive", "b2.jpg", "b10.jpg", "notes.txt"]);
});

test("a number column starts biggest first, and empty values stay last", () => {
  const t = useTable(files(), options);
  t.by("size", true);
  assert.deepEqual(names(t), ["b2.jpg", "b10.jpg", "Archive", "notes.txt"]);
  t.by("size", true);
  assert.deepEqual(names(t), ["Archive", "b10.jpg", "b2.jpg", "notes.txt"], "flipped, empty still last");
});

test("pinned rows stay on top whatever the sort", () => {
  const t = useTable(files(), { ...options, pinned: (r) => r.dir });
  t.by("size", true);
  assert.equal(names(t)[0], "Archive");
});

test("search and a chip narrow together, and clearing brings everything back", () => {
  const t = useTable(files(), options);
  t.query.value = "B1";
  assert.deepEqual(names(t), ["b10.jpg"], "case does not matter");
  t.query.value = "";
  t.pick("kind", "photo");
  assert.deepEqual(names(t), ["b2.jpg", "b10.jpg"]);
  assert.equal(t.narrowed.value, true);
  t.pick("kind", "photo");
  assert.equal(t.shown.value.length, 4, "choosing a chip again lets it go");
  t.query.value = "zzz";
  t.clear();
  assert.equal(t.shown.value.length, 4);
});

test("chip counts are over every row, so a choice never zeroes the others", () => {
  const t = useTable(files(), options);
  t.pick("kind", "text");
  const kinds = t.facetValues.value[0].values;
  assert.deepEqual(kinds[0], { value: "photo", count: 2 });
});

test("a table follows its rows when they change", () => {
  const rows = files();
  const t = useTable(rows, options);
  rows.value = rows.value.slice(0, 1);
  assert.deepEqual(names(t), ["b10.jpg"]);
});
