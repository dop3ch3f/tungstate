// Sizes and plurals, which every screen prints.
import { test } from "node:test";
import assert from "node:assert/strict";
import { bytes, plural } from "../src/lib/format.ts";

test("a size never shows four digits", () => {
  // 1,000 MiB used to read "1000 MB".
  assert.equal(bytes(1_048_576_000), "1.0 GB");
  assert.equal(bytes(1000), "1.0 KB");
  assert.equal(bytes(999), "999 B");
});

test("sizes keep their usual shape either side of the step", () => {
  assert.equal(bytes(2_516_582), "2.4 MB");
  assert.equal(bytes(125_829_120), "120 MB");
  assert.equal(bytes(3_435_973_836), "3.2 GB");
});

test("one of a thing is singular, and the plural can be irregular", () => {
  assert.equal(plural(1, "group"), "1 group");
  assert.equal(plural(3, "group"), "3 groups");
  assert.equal(plural(0, "copy", "copies"), "0 copies");
});
