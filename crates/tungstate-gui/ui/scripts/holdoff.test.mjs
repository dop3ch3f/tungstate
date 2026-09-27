// "Stop now" appears where "Stop after these files" was clicked; the second
// click of a double click used to land on it.
import { test } from "node:test";
import assert from "node:assert/strict";
import { holdoff } from "../src/lib/holdoff.ts";

test("a control ignores clicks for a moment after it appears", () => {
  let t = 1000;
  const h = holdoff(600, () => t);
  assert.ok(h.ready(), "ready before it has ever appeared");
  h.start();
  t += 150; // the second click of a double click
  assert.ok(!h.ready());
  t += 450;
  assert.ok(h.ready(), "and takes a deliberate click after that");
});
