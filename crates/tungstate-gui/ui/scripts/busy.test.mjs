// The guard every engine-calling button now goes through.
import "./fake-engine.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { useBusy } from "../src/state/useBusy.ts";

const later = (ms, value) => new Promise((r) => setTimeout(() => r(value), ms));

test("a second click under the same key while the first runs does nothing", async () => {
  const b = useBusy();
  let calls = 0;
  const work = () => (calls++, later(20, "done"));
  const [first, second] = await Promise.all([b.run("run", work), b.run("run", work)]);
  assert.equal(calls, 1);
  assert.equal(first, "done");
  assert.equal(second, null);
  assert.ok(!b.busy("run"), "and it is free again afterwards");
});

test("different keys run side by side and each says it is busy", async () => {
  const b = useBusy();
  const one = b.run(1, () => later(20));
  const two = b.run(2, () => later(20));
  await later(5);
  assert.ok(b.busy(1) && b.busy(2) && b.busy());
  await Promise.all([one, two]);
  assert.ok(!b.busy());
});

test("a failure lands in the one error slot and frees the key", async () => {
  const b = useBusy();
  const out = await b.run("x", () => Promise.reject("the NAS is out of reach"));
  assert.equal(out, null);
  assert.equal(b.problem.value, "the NAS is out of reach");
  assert.ok(!b.busy("x"));
});
