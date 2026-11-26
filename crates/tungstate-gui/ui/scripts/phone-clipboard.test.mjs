// The phone's clipboard and windows, against a fake engine.
import { engine } from "./fake-engine.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";

// Node has no localStorage of its own; the stores remember choices in it.
const kept = new Map();
globalThis.localStorage ??= { getItem: (k) => kept.get(k) ?? null, setItem: (k, v) => kept.set(k, String(v)), removeItem: (k) => kept.delete(k) };

const { requestFor, useClipboard } = await import("../src/phone/state/useClipboard.ts");
const { useWindows } = await import("../src/phone/state/useWindows.ts");

test("a move removes originals only through the engine's checked move", () => {
  const req = requestFor("move", ["/s/DCIM/a.jpg"], "Area51:In");
  assert.equal(req.source_policy, "delete");
  assert.equal(req.verify, "readback");
  assert.equal(req.on_conflict, "rename");
  assert.equal(requestFor("copy", ["/s/DCIM/a.jpg"], "Area51:In").source_policy, "keep");
});

test("pasting starts one transfer for the whole clipboard and empties it", async () => {
  const sent = [];
  engine.answer = async (cmd, args) => {
    if (cmd === "start_transfer") {
      sent.push(args.request);
      return { started: true, waiting: 0, job: 7 };
    }
  };
  const c = useClipboard();
  c.hold({ op: "move", what: "2 photos", bytes: 10, from: "Camera", paths: ["/s/DCIM/a.jpg", "/s/Pictures/b.jpg"] });
  await c.paste("Area51:Photos", "Photos");
  assert.equal(sent.length, 1);
  assert.equal(sent[0].legs.length, 2, "one leg per source folder");
  assert.equal(c.held.value, false);
  assert.deepEqual(c.started.value, { job: 7, op: "move", what: "2 photos", to: "Photos", destination: "Area51:Photos", paths: ["/s/DCIM/a.jpg", "/s/Pictures/b.jpg"] });
});

test("a refused paste keeps the clipboard, so nothing chosen is lost", async () => {
  engine.answer = async () => { throw "the destination is inside the source"; };
  const c = useClipboard();
  c.hold({ op: "move", what: "a folder", bytes: 0, from: "DCIM", paths: ["/s/DCIM"] });
  await assert.rejects(c.paste("/s/DCIM/inner", "inner"));
  assert.equal(c.held.value, true);
  c.drop();
});

test("Free up's findings land their own way, not as a transfer", async () => {
  const landed = [];
  engine.calls.length = 0;
  const c = useClipboard();
  c.hold({ op: "move", what: "3,213 items", bytes: 1, from: "Free up", paths: [], land: async (to) => void landed.push(to) });
  await c.paste("Area51:Photos/Phone", "Phone");
  assert.deepEqual(landed, ["Area51:Photos/Phone"]);
  assert.ok(!engine.calls.some((x) => x.cmd === "start_transfer"));
});

test("ticking is per window, and a second window opens beside the first", () => {
  const w = useWindows();
  w.go({ kind: "folder", path: "/s/DCIM" });
  w.toggle("/s/DCIM/a.jpg");
  assert.equal(w.selecting.value, true);
  w.second({ kind: "folder", path: "Area51:" });
  assert.equal(w.active.value, 1);
  assert.equal(w.selecting.value, false, "the new window has nothing ticked");
  w.switchTo(0);
  assert.equal(w.win.value.picked.size, 1);
  w.close(1);
  assert.equal(w.wins.value.length, 1);
  assert.equal(w.active.value, 0);
});

const { tally, unfinished } = await import("../src/phone/lib/progress.ts");
const { nameOf } = await import("../src/phone/lib/paths.ts");

test("sent, checked and failed come from each file's own state", () => {
  const t = tally([
    { path: "a", size: 10, state: "transferred", done: 10 },
    { path: "b", size: 10, state: "checking", done: 10 },
    { path: "c", size: 10, state: "live", done: 4 },
    { path: "d", size: 10, state: "waiting", done: 0 },
    { path: "e", size: 10, state: "failed", done: 0 },
  ]);
  assert.deepEqual(t, { sent: 3, checked: 1, failed: 1, count: 5, bytes: 34, total: 50 });
});

test("after a pause only what was not checked is sent again", () => {
  const rows = [
    { path: "a.jpg", size: 1, state: "transferred", done: 1 },
    { path: "Trip/1.jpg", size: 1, state: "transferred", done: 1 },
    { path: "Trip/2.jpg", size: 1, state: "waiting", done: 0 },
  ];
  const chosen = ["/s/DCIM/a.jpg", "/s/DCIM/Trip", "/s/DCIM/b.jpg"];
  assert.deepEqual(unfinished(chosen, rows, nameOf), ["/s/DCIM/Trip", "/s/DCIM/b.jpg"]);
});

const { byYear, yearState } = await import("../src/phone/lib/findings.ts");

test("findings group by year in the engine's order, counting what is ticked", () => {
  const a = (id, year, size) => ({ id, name: `${id}.jpg`, size, takenMs: 0, year, kind: "photo", favourite: false });
  const groups = byYear([a("1", 2026, 5), a("2", 2026, 7), a("3", 2025, 1)], new Set(["2"]));
  assert.deepEqual(groups.map((g) => [g.year, g.items.length, g.ticked, g.tickedBytes]), [[2026, 2, 1, 5], [2025, 1, 1, 1]]);
  assert.equal(yearState(groups[0]), "some");
  assert.equal(yearState(groups[1]), "all");
});
