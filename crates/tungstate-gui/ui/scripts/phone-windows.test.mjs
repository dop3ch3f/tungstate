// The phone's window tabs: where a storage or connection opens, and what
// closing one leaves in front. The store is module-level, so each test starts
// from one tab on Files home; node runs a file's tests one after another.
import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";

const kept = new Map();
globalThis.localStorage ??= { getItem: (k) => kept.get(k) ?? null, setItem: (k, v) => kept.set(k, String(v)), removeItem: (k) => kept.delete(k) };

const { useWindows } = await import("../src/phone/state/useWindows.ts");
const w = useWindows();
const folder = (path) => ({ kind: "folder", path });
const places = () => w.wins.value.map((x) => (x.place.kind === "folder" ? x.place.path : x.place.kind));

beforeEach(() => {
  while (w.wins.value.length > 1) w.close(w.wins.value.length - 1);
  w.switchTo(0);
  w.go({ kind: "home" });
});

test("a storage opened from a blank tab takes that tab", () => {
  w.openTab(folder("/storage/emulated/0"));
  assert.deepEqual(places(), ["/storage/emulated/0"]);
});

test("each storage and connection gets a tab of its own", () => {
  w.openTab(folder("/storage/emulated/0"));
  w.openTab(folder("Area51:"));
  assert.deepEqual(places(), ["/storage/emulated/0", "Area51:"]);
  assert.equal(w.active.value, 1);
});

test("opening a place already in a tab goes back to that tab, where it was", () => {
  w.openTab(folder("/storage/emulated/0"));
  w.go(folder("/storage/emulated/0/DCIM"));
  w.openTab(folder("Area51:"));
  w.go(folder("Area51:Photos/2026"));
  w.openTab(folder("/storage/emulated/0"));
  assert.equal(w.active.value, 0);
  assert.equal(w.win.value.place.path, "/storage/emulated/0/DCIM", "left where it was");
  w.openTab(folder("Area51:"));
  assert.equal(w.active.value, 1);
  assert.equal(w.wins.value.length, 2);
});

test("a card is not mistaken for internal storage by a shared prefix", () => {
  w.openTab(folder("/storage/emulated/0"));
  w.openTab(folder("/storage/emulated/01"));
  assert.equal(w.wins.value.length, 2);
});

test("with every tab open, a new place opens in the current one", () => {
  w.openTab(folder("/a"));
  for (const p of ["/b", "/c", "/d", "/e"]) w.openTab(folder(p));
  assert.equal(w.wins.value.length, w.max);
  w.openTab(folder("Area51:"));
  assert.equal(w.wins.value.length, w.max);
  assert.equal(w.win.value.place.path, "Area51:");
  w.newTab();
  assert.equal(w.wins.value.length, w.max, "+ does nothing when full");
});

test("+ opens Files home in a new tab", () => {
  w.openTab(folder("/a"));
  w.newTab();
  assert.deepEqual(places(), ["/a", "home"]);
  assert.equal(w.active.value, 1);
});

test("closing a tab before the current one keeps the same tab in front", () => {
  w.openTab(folder("/a"));
  w.openTab(folder("/b"));
  w.openTab(folder("/c"));
  w.switchTo(1);
  w.close(0);
  assert.equal(w.win.value.place.path, "/b");
  w.switchTo(1);
  w.close(w.active.value);
  assert.equal(w.win.value.place.path, "/b", "closing the last tab shows the one before");
  w.close(0);
  assert.equal(w.wins.value.length, 1, "the last tab never closes");
});

test("after a connection's folder changes, its tabs go back to its top", () => {
  w.openTab(folder("Area51:"));
  w.go(folder("Area51:Photos/2026"));
  w.openTab(folder("/storage/emulated/0"));
  w.go(folder("/storage/emulated/0/DCIM"));
  w.rehome("Area51:");
  assert.deepEqual(places(), ["Area51:", "/storage/emulated/0/DCIM"], "other tabs stay where they are");
});
