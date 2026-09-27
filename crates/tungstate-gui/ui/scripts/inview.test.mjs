// Which rows of a long list are drawn.
import { test } from "node:test";
import assert from "node:assert/strict";
import { inView } from "../src/lib/inview.ts";

const rows = (settled, busy) => [
  ...Array.from({ length: settled }, (_, i) => ({ i, busy: false })),
  ...Array.from({ length: busy }, (_, i) => ({ i: settled + i, busy: true })),
];
const isBusy = (r) => r.busy;

test("a short list is drawn whole", () => {
  const all = rows(5, 5);
  assert.deepEqual(inView(all, isBusy, true, 200), { from: 0, rows: all });
});

test("a long running list is drawn where the work is, with a little of what finished", () => {
  const { from, rows: shown } = inView(rows(5000, 5000), isBusy, true, 200);
  assert.equal(from, 4980);
  assert.equal(shown.length, 200);
  assert.equal(shown[20].busy, true, "the first busy row, after twenty settled ones");
});

test("near the end the slice stays full rather than running off the list", () => {
  const { from, rows: shown } = inView(rows(9990, 10), isBusy, true, 200);
  assert.equal(from, 9800);
  assert.equal(shown.length, 200);
});

test("a sorted or filtered list starts at the top, and a finished one too", () => {
  assert.equal(inView(rows(5000, 5000), isBusy, false, 200).from, 0);
  assert.equal(inView(rows(10000, 0), isBusy, true, 200).from, 0);
});
