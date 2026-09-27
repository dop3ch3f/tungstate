// Updating from inside the app, against a fake engine and a fake updater.
import { engine } from "./fake-engine.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { useUpdate } from "../src/state/useUpdate.ts";

const release = { rid: 1, currentVersion: "0.1.0-alpha.4", version: "0.1.0-alpha.5", body: "What changed." };

test("an update waits for a running transfer, then installs and restarts", async () => {
  engine.calls.length = 0;
  let busyFor = 2;
  engine.answer = async (cmd) => {
    if (cmd === "plugin:updater|check") return release;
    if (cmd === "plugin:updater|download") return 7;
    if (cmd === "ready_to_restart") return busyFor-- > 0 ? "transfer" : null;
    return null;
  };
  const u = useUpdate();
  await u.look();
  assert.equal(u.ready.value.version, "0.1.0-alpha.5");

  const seen = [];
  const applying = u.apply({ every: 5 });
  const watch = setInterval(() => seen.push(u.waitingFor.value), 1);
  await applying;
  clearInterval(watch);

  const order = engine.calls.map((c) => c.cmd).filter((c) => !c.startsWith("plugin:resources"));
  assert.deepEqual(order.slice(-6), [
    "plugin:updater|download",
    "ready_to_restart",
    "ready_to_restart",
    "ready_to_restart",
    "plugin:updater|install",
    "plugin:process|restart",
  ]);
  assert.ok(seen.includes("transfer"), "it said what it was waiting for");
});

test("an install that fails keeps syncs in step again and says why", async () => {
  engine.calls.length = 0;
  engine.answer = async (cmd) => {
    if (cmd === "plugin:updater|check") return { ...release, rid: 2, version: "0.1.0-alpha.6" };
    if (cmd === "plugin:updater|download") return 8;
    if (cmd === "plugin:updater|install") throw "the app folder is read-only";
    return null;
  };
  const u = useUpdate();
  // The first test restarted, which in a real window ends the process.
  u.step.value = "idle";
  u.ready.value = null;
  await u.look();
  await u.apply({ every: 5 });
  assert.equal(u.problem.value, "the app folder is read-only");
  assert.equal(u.step.value, "idle");
  assert.ok(engine.calls.some((c) => c.cmd === "not_restarting"));
});
