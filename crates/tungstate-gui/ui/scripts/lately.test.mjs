// Home's "Lately": one sentence per burst of alike ops, and nothing the
// watcher already said said twice.
import { test } from "node:test";
import assert from "node:assert/strict";
import { lately } from "../src/lib/lately.ts";

const op = (id, over) => ({
  id, status: "ok", kind: "rename", source: `/u/Downloads/a${id}.txt`, destination: `/u/Downloads/docs/a${id}.txt`,
  size: 1, hash: null, link: null, note: null, started_at: 1_000_000 - id, ...over,
});
const none = new Map();

test("a burst of alike ops is one line that counts files", () => {
  const lines = lately([op(1), op(2), op(3)], [], none, 5);
  assert.deepEqual(lines.map((l) => l.text), ["Renamed 3 files in Downloads"]);
});

test("one op names its file", () => {
  assert.equal(lately([op(1)], [], none, 5)[0].text, "Renamed a1.txt in Downloads");
});

test("a failure is its own line, in the failure tone", () => {
  const lines = lately([op(1), op(2, { status: "failed" }), op(3)], [], none, 5);
  assert.deepEqual(lines.map((l) => [l.text, l.tone]), [
    ["Renamed a1.txt in Downloads", "ok"],
    ["Could not rename a2.txt in Downloads", "bad"],
    ["Renamed a3.txt in Downloads", "ok"],
  ]);
});

test("ops far apart in time are separate lines", () => {
  const lines = lately([op(1), op(2, { started_at: 1_000_000 - 11 * 60_000 })], [], none, 5);
  assert.equal(lines.length, 2);
});

test("a transfer says where it went, and folders count as folders", () => {
  const sent = [1, 2].map((i) => op(i, { kind: "move", link: "to-nas", source: `/u/out/v${i}.mp4`, destination: `area51:media/incoming/v${i}.mp4` }));
  const made = [3, 4].map((i) => op(i, { kind: "mkdir", source: null, destination: `/u/Downloads/d${i}` }));
  assert.deepEqual(lately([...sent, ...made], [], none, 5).map((l) => l.text), [
    "Moved 2 files to incoming",
    "Made 2 folders in Downloads",
  ]);
});

test("ops the watcher already announced are not said again", () => {
  const notice = { kind: "tidied", folder: "Downloads", files: 3, plan: 9, why: "", at: 1_000_000 };
  const lines = lately([op(1), op(2), op(3)], [notice], new Map([["Downloads", "/u/Downloads"]]), 5);
  assert.deepEqual(lines.map((l) => [l.text, l.by]), [["Tidied 3 files in Downloads", "watcher"]]);
});

test("the newest lines come first, cut to the limit", () => {
  const ops = [1, 2, 3, 4].map((i) => op(i, { kind: i % 2 ? "rename" : "move", started_at: 1_000_000 - i * 20 * 60_000 }));
  const lines = lately(ops, [], none, 2);
  assert.equal(lines.length, 2);
  assert.ok(lines[0].at > lines[1].at);
});
