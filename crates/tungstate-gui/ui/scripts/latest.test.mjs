// Answering two conflicts quickly previewed twice, and whichever answer came
// back last was shown, even when it was for the first click.
import { test } from "node:test";
import assert from "node:assert/strict";
import { latest } from "../src/lib/latest.ts";

test("an older answer arriving last is stale", async () => {
  const requests = latest();
  const shown = [];
  const ask = async (label, delay) => {
    const ticket = requests.take();
    await new Promise((r) => setTimeout(r, delay));
    if (!requests.stale(ticket)) shown.push(label);
  };
  await Promise.all([ask("first click", 30), ask("second click", 5)]);
  assert.deepEqual(shown, ["second click"]);
});
