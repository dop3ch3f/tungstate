// Remembering the last listing of each place, within a budget of rows.
import { test } from "node:test";
import assert from "node:assert/strict";
import { makeShelf } from "../src/phone/lib/shelf.ts";

const rows = (n) => Array.from({ length: n }, (_, i) => i);

test("a place looked at before comes back as it was", () => {
  const shelf = makeShelf(100);
  shelf.put("/storage/emulated/0/DCIM", rows(3));
  assert.deepEqual(shelf.get("/storage/emulated/0/DCIM"), [0, 1, 2]);
  assert.equal(shelf.get("/storage/emulated/0/Music"), undefined);
});

test("a fresh listing replaces the old one without counting it twice", () => {
  const shelf = makeShelf(100);
  shelf.put("a", rows(40));
  shelf.put("a", rows(10));
  assert.equal(shelf.held(), 10);
  assert.equal(shelf.get("a").length, 10);
});

test("over budget, the place looked at longest ago goes first", () => {
  const shelf = makeShelf(100);
  shelf.put("a", rows(40));
  shelf.put("b", rows(40));
  shelf.get("a"); // a is now the most recent
  shelf.put("c", rows(40));
  assert.equal(shelf.get("b"), undefined);
  assert.ok(shelf.get("a"));
  assert.ok(shelf.get("c"));
  assert.equal(shelf.held(), 80);
});

test("a listing bigger than the whole budget is not kept, and evicts nothing", () => {
  const shelf = makeShelf(100);
  shelf.put("a", rows(40));
  shelf.put("huge", rows(500));
  assert.equal(shelf.get("huge"), undefined);
  assert.ok(shelf.get("a"));
  assert.equal(shelf.held(), 40);
});
