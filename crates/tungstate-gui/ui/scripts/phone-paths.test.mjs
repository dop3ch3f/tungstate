// Where a location's folder and name are, as the phone's file windows read them.
import { test } from "node:test";
import assert from "node:assert/strict";
import { childOf, connectionOf, crumbs, legsFor, nameOf, parentOf, topOf } from "../src/phone/lib/paths.ts";

const tops = [
  { label: "Internal storage", path: "/storage/emulated/0" },
  { label: "SD card", path: "/storage/1A2B-3C4D" },
];

test("a connection's folder and name split on / after its name", () => {
  assert.equal(connectionOf("Area51:Photos/2026"), "Area51");
  assert.equal(connectionOf("/storage/emulated/0"), null);
  assert.equal(parentOf("Area51:Photos/2026"), "Area51:Photos");
  assert.equal(parentOf("Area51:Photos"), "Area51:");
  assert.equal(parentOf("Area51:"), null);
  assert.equal(nameOf("Area51:Photos/2026/"), "2026");
});

test("a phone path's folder and name, up to the root", () => {
  assert.equal(parentOf("/storage/emulated/0/DCIM/a.jpg"), "/storage/emulated/0/DCIM");
  assert.equal(parentOf("/top"), "/");
  assert.equal(parentOf("/"), null);
  assert.equal(nameOf("/storage/emulated/0/DCIM"), "DCIM");
});

test("children join the way the engine joins them", () => {
  assert.equal(childOf("Area51:", "Photos"), "Area51:Photos");
  assert.equal(childOf("Area51:Photos", "2026"), "Area51:Photos/2026");
  assert.equal(childOf("/storage/emulated/0/", "DCIM"), "/storage/emulated/0/DCIM");
});

test("the deepest volume holds a path, not one that merely shares its start", () => {
  assert.equal(topOf("/storage/emulated/0/DCIM", tops)?.label, "Internal storage");
  assert.equal(topOf("/storage/emulated/01", tops), null);
  assert.equal(topOf("Area51:x", tops)?.path, "Area51:");
});

test("chips run from the place's top to the folder above this one", () => {
  const labels = (p) => crumbs(p, tops).map((c) => c.label);
  assert.deepEqual(labels("/storage/emulated/0/DCIM/Camera"), ["Internal storage", "DCIM"]);
  assert.deepEqual(labels("/storage/emulated/0/DCIM"), ["Internal storage"]);
  assert.deepEqual(labels("/storage/emulated/0"), []);
  assert.deepEqual(labels("Area51:Photos/2026"), ["Area51", "Photos"]);
  assert.deepEqual(labels("Area51:"), []);
});

test("a selection spanning folders becomes one leg per folder, in order", () => {
  const legs = legsFor(["/s/DCIM/a.jpg", "/s/Download/b.pdf", "/s/DCIM/c.jpg"], "Area51:In");
  assert.deepEqual(legs, [
    { source: "/s/DCIM", destination: "Area51:In", names: ["a.jpg", "c.jpg"] },
    { source: "/s/Download", destination: "Area51:In", names: ["b.pdf"] },
  ]);
});

test("a place reads as its chips and its own name", async () => {
  const { placeName } = await import("../src/phone/lib/paths.ts");
  assert.equal(placeName("Area51:Photos/2026", tops), "Area51 › Photos › 2026");
  assert.equal(placeName("Area51:", tops), "Area51");
  assert.equal(placeName("/storage/emulated/0/DCIM", tops), "Internal storage › DCIM");
  assert.equal(placeName("/storage/emulated/0", tops), "Internal storage");
});
