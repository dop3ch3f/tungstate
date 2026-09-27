// Which look a stored choice and the system setting come to.
import { test } from "node:test";
import assert from "node:assert/strict";
import { choiceOf, resolve } from "../src/lib/theme.ts";

test("matching the system is retro, light or dark with it", () => {
  assert.deepEqual(resolve("system", false), { theme: "retro", tone: "light" });
  assert.deepEqual(resolve("system", true), { theme: "retro", tone: "dark" });
});

test("a pinned theme ignores the system", () => {
  assert.deepEqual(resolve("paper", true), { theme: "paper", tone: "light" });
  assert.deepEqual(resolve("graphite", false), { theme: "graphite", tone: "dark" });
  assert.deepEqual(resolve("retro-dark", false), { theme: "retro", tone: "dark" });
});

test("nothing stored, or something this version does not know, is the system", () => {
  assert.equal(choiceOf(null), "system");
  assert.equal(choiceOf("neon"), "system");
  assert.equal(choiceOf("paper"), "paper");
});
