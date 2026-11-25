// Android's back button: the newest step that can go back does.
import { test } from "node:test";
import assert from "node:assert/strict";
import { addStep, stepBack } from "../src/phone/lib/back.ts";

test("the top step goes back first, and a finished one passes it down", () => {
  const seen = [];
  const window = addStep(() => (seen.push("window"), true));
  let sheetOpen = true;
  const sheet = addStep(() => (sheetOpen ? (seen.push("sheet"), (sheetOpen = false), true) : false));
  assert.equal(stepBack(), true);
  assert.equal(stepBack(), true);
  assert.deepEqual(seen, ["sheet", "window"]);
  sheet();
  window();
  assert.equal(stepBack(), false, "nothing left: the app may close");
});
