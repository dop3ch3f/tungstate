// The Sync section's state, against a fake engine.
import { engine } from "./fake-engine.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { attachSyncStream, useSync } from "../src/state/useSync.ts";

test("a run kept in step does not write into the result of a run you pressed", async () => {
  // Found driving the real window: a background run's files and bytes
  // appeared above the summary of the earlier run being looked at.
  await attachSyncStream();
  const s = useSync();
  s.current.value = { name: "capcut" };
  s.phase.value = "done";
  s.ran.value = { sync: "capcut", plan: 3 };
  s.rows.value = [];

  engine.emit("sync://leg", { index: 0, from: "laptop", to: "nas" });
  engine.emit("sync://planned", [{ path: "third.mp4", size: 13 }]);
  engine.emit("sync://finished", { path: "third.mp4", outcome: "transferred", detail: null });

  assert.deepEqual(s.rows.value, []);
  assert.equal(s.leg.value, null);
});
