// Sizes and plurals, which every screen prints.
import { test } from "node:test";
import assert from "node:assert/strict";
import { bytes, ordinal, plural } from "../src/lib/format.ts";

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

test("a place in line reads the way it is said", () => {
  assert.deepEqual([1, 2, 3, 4].map(ordinal), ["1st", "2nd", "3rd", "4th"]);
  assert.deepEqual([11, 12, 13, 21, 22, 111].map(ordinal), ["11th", "12th", "13th", "21st", "22nd", "111th"]);
});

test("ago says a time the way a person would", async () => {
  const { ago } = await import("../src/lib/format.ts");
  const now = 10 * 86_400_000;
  assert.equal(ago(now - 20_000, now), "just now");
  assert.equal(ago(now - 5 * 60_000, now), "5 min ago");
  assert.equal(ago(now - 61 * 60_000, now), "an hour ago");
  assert.equal(ago(now - 3 * 3_600_000, now), "3 hours ago");
  assert.equal(ago(now - 30 * 3_600_000, now), "yesterday");
  assert.equal(ago(now - 4 * 86_400_000, now), "4 days ago");
  assert.doesNotMatch(ago(now - 9 * 86_400_000, now), /ago/);
});
