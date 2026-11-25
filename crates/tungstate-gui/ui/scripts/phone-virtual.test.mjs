// Drawing only the rows on screen, and remembering a view chosen before.
import { test } from "node:test";
import assert from "node:assert/strict";
import { rowsInView } from "../src/phone/lib/virtual.ts";

test("only the rows on screen, and a few either side, are drawn", () => {
  assert.deepEqual(rowsInView(0, 800, 88, 5000, 6), [0, 16]);
  assert.deepEqual(rowsInView(88 * 1000, 800, 88, 5000, 6), [994, 1016]);
  assert.deepEqual(rowsInView(88 * 4999, 800, 88, 5000, 6), [4993, 5000]);
  assert.deepEqual(rowsInView(0, 800, 88, 0), [0, 0]);
});

import { storedView } from "../src/phone/lib/views.ts";

test("a view chosen in the two-view build is kept, and new installs show previews", () => {
  assert.equal(storedView("grid"), "tiles");
  assert.equal(storedView("list"), "list");
  assert.equal(storedView(null), "detailed");
  assert.equal(storedView("nonsense"), "detailed");
});
