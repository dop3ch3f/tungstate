// The transfer queue's state, against a fake engine.
//
// One module-level store, as in the window, so each test starts by clearing
// what it reads. node:test runs a file's top-level tests one after another.
import { engine } from "./fake-engine.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { attachTransferStream, useTransfer } from "../src/state/useTransfer.ts";

engine.answer = async (cmd) => (cmd === "transfer_queue" ? { running: null, waiting: [] } : null);
await attachTransferStream();
const t = useTransfer();

const job = (id, over = {}) => ({
  id, name: null, source: `/from/${id}`, destination: `/to/${id}`,
  exchange: false, removes_originals: true, files: null, bytes: null, ...over,
});
const summary = (over = {}) => ({
  transferred: 0, already_present: 0, skipped: 0, quarantined: 0, failed: 0, bytes: 0,
  recovered: 0, pruned: 0, cancelled: false, destination_lost: false, failures: [], ...over,
});
const fresh = () => {
  t.finished.value = [];
  t.placed.value = null;
};
/** One file planned, sent and verified. */
function sendOne(path, size) {
  engine.emit("transfer://started", { path, size });
  engine.emit("transfer://finished", { path, outcome: "transferred", detail: null });
}

test("each transfer keeps its own files, however many ran one after another", () => {
  // Found as "behaved weird": the second transfer's plan replaced the first
  // one's list, and the summary added the two together.
  fresh();
  engine.emit("transfer://job", job(1));
  engine.emit("transfer://planned", [{ path: "clip-1.mov", size: 10 }, { path: "clip-2.mov", size: 20 }]);
  sendOne("clip-1.mov", 10);
  sendOne("clip-2.mov", 20);
  engine.emit("transfer://done", { job: 1, ...summary({ transferred: 2, bytes: 30 }) });

  engine.emit("transfer://job", job(2, { removes_originals: false }));
  engine.emit("transfer://planned", [{ path: "IMG_0001.jpg", size: 3 }]);
  sendOne("IMG_0001.jpg", 3);
  engine.emit("transfer://done", { job: 2, ...summary({ transferred: 1, bytes: 3 }) });

  const [second, first] = t.finished.value;
  assert.deepEqual(first.rows.map((r) => r.path), ["clip-1.mov", "clip-2.mov"]);
  assert.equal(first.summary.transferred, 2);
  assert.deepEqual(second.rows.map((r) => r.path), ["IMG_0001.jpg"]);
  assert.equal(second.summary.bytes, 3);
  assert.equal(t.current.value, null);
  assert.deepEqual(t.rows.value, []);
});

test("joining the queue leaves the running transfer's files on screen and says where it is", () => {
  fresh();
  engine.emit("transfer://job", job(3));
  engine.emit("transfer://planned", [{ path: "big.mov", size: 100 }]);
  engine.emit("transfer://started", { path: "big.mov", size: 100 });

  t.accepted({ started: false, waiting: 1, job: 4 });
  engine.emit("transfer://queue", { running: job(3), waiting: [job(4)] });

  assert.equal(t.placed.value.line, "Added to the queue, 2nd in line.");
  assert.deepEqual(t.rows.value.map((r) => [r.path, r.state]), [["big.mov", "live"]]);
  assert.deepEqual(t.queue.value.waiting.map((j) => j.id), [4]);

  engine.emit("transfer://finished", { path: "big.mov", outcome: "transferred", detail: null });
  engine.emit("transfer://done", { job: 3, ...summary({ transferred: 1, bytes: 100 }) });
  engine.emit("transfer://job", job(4));
  assert.equal(t.placed.value, null, "the notice is spent once it starts");
});

test("a started transfer needs no notice", () => {
  fresh();
  t.accepted({ started: true, waiting: 0, job: 9 });
  assert.equal(t.placed.value, null);
});

test("an exchange's second direction adds to the list rather than replacing the first", () => {
  fresh();
  engine.emit("transfer://job", job(5, { exchange: true }));
  engine.emit("transfer://planned", [{ path: "notes.txt", size: 1 }]);
  sendOne("notes.txt", 1);
  // The same name coming back the other way.
  engine.emit("transfer://planned", [{ path: "notes.txt", size: 2 }]);
  engine.emit("transfer://started", { path: "notes.txt", size: 2 });

  assert.deepEqual(t.rows.value.map((r) => [r.path, r.state]), [
    ["notes.txt", "transferred"],
    ["notes.txt", "live"],
  ]);
});

test("a transfer that fails keeps its reason in its own row, and the next starts clean", () => {
  fresh();
  engine.emit("transfer://job", job(6));
  engine.emit("transfer://planned", [{ path: "a.mov", size: 1 }]);
  engine.emit("transfer://error", { job: 6, message: "the far side refused the connection" });
  engine.emit("transfer://job", job(7));

  const [failed] = t.finished.value;
  assert.equal(failed.job.id, 6);
  assert.equal(failed.summary, null);
  assert.equal(failed.problem, "the far side refused the connection");
  assert.deepEqual(failed.rows.map((r) => r.path), ["a.mov"]);
  assert.equal(t.current.value.id, 7);
  assert.deepEqual(t.rows.value, []);
  assert.equal(t.problem.value, null, "one transfer's failure is not the window's");
});

test("only a handful of finished transfers are kept", () => {
  fresh();
  for (let id = 100; id < 112; id++) {
    engine.emit("transfer://job", job(id));
    engine.emit("transfer://done", { job: id, ...summary() });
  }
  assert.equal(t.finished.value.length, 8);
  assert.equal(t.finished.value[0].job.id, 111, "newest first");
});

test("an emptied queue ends a stop", () => {
  t.stopping.value = true;
  engine.emit("transfer://queue", { running: null, waiting: [] });
  assert.equal(t.stopping.value, false);
});

test("taking a transfer out of the queue asks the engine for that one", async () => {
  engine.calls.length = 0;
  t.accepted({ started: false, waiting: 2, job: 12 });
  await t.unqueue(12);
  const asked = engine.calls.find((c) => c.cmd === "remove_from_queue");
  assert.deepEqual(asked.args, { job: 12 });
  assert.equal(t.placed.value, null);
});
