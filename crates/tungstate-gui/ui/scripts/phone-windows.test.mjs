// The phone's window tabs: home stays home, where a connection or card opens,
// and what closing one leaves in front. The store is module-level, so each
// test starts from the home tab alone on Files home; node runs a file's tests
// one after another.
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

test("a connection opened from home gets its own tab, and home stays home", () => {
  w.openTab(folder("Area51:"));
  assert.deepEqual(places(), ["home", "Area51:"]);
  assert.equal(w.active.value, 1);
});

test("home itself goes into the phone's storage", () => {
  w.go(folder("/storage/emulated/0"));
  w.openTab(folder("FTP-Photos:"));
  assert.deepEqual(places(), ["/storage/emulated/0", "FTP-Photos:"]);
});

test("opening a place already in a tab goes back to that tab, where it was", () => {
  w.openTab(folder("Area51:"));
  w.go(folder("Area51:Photos/2026"));
  w.switchTo(0);
  w.openTab(folder("FTP-Photos:"));
  w.switchTo(0);
  w.openTab(folder("Area51:"));
  assert.equal(w.active.value, 1);
  assert.equal(w.win.value.place.path, "Area51:Photos/2026", "left where it was");
  assert.equal(w.wins.value.length, 3);
});

test("a blank tab from + takes what is opened in it, even a place open elsewhere", () => {
  w.openTab(folder("Area51:"));
  w.newTab();
  w.openTab(folder("Area51:"));
  assert.deepEqual(places(), ["home", "Area51:", "Area51:"], "two tabs on one place, to copy between its folders");
  assert.equal(w.active.value, 2);
});

test("a folder deep in a connection reuses only a tab already in that folder", () => {
  w.openTab(folder("Area51:"));
  w.switchTo(0);
  w.openTab(folder("Area51:Photos"));
  assert.equal(w.wins.value.length, 3, "Area51's top is not inside Photos");
  w.switchTo(0);
  w.openTab(folder("Area51:Photos"));
  assert.equal(w.active.value, 2);
  assert.equal(w.wins.value.length, 3);
});

test("a card is not mistaken for another volume by a shared prefix", () => {
  w.openTab(folder("/storage/1A2B"));
  w.switchTo(0);
  w.openTab(folder("/storage/1A2B-3C"));
  assert.equal(w.wins.value.length, 3);
});

test("with every tab open, the last one gives way, never home", () => {
  for (const p of ["A:", "B:", "C:", "D:"]) {
    w.switchTo(0);
    w.openTab(folder(p));
  }
  assert.equal(w.wins.value.length, w.max);
  w.switchTo(0);
  w.openTab(folder("E:"));
  assert.deepEqual(places(), ["home", "A:", "B:", "C:", "E:"]);
  assert.equal(w.active.value, 4);
  w.switchTo(2);
  w.openTab(folder("F:"));
  assert.equal(places()[2], "F:", "from a tab in use, that tab gives way");
  w.newTab();
  assert.equal(w.wins.value.length, w.max, "+ does nothing when full");
});

test("+ opens Files home in a new tab", () => {
  w.openTab(folder("A:"));
  w.newTab();
  assert.deepEqual(places(), ["home", "A:", "home"]);
  assert.equal(w.active.value, 2);
});

test("closing a tab before the current one keeps the same tab in front", () => {
  for (const p of ["A:", "B:", "C:"]) {
    w.switchTo(0);
    w.openTab(folder(p));
  }
  w.switchTo(2);
  w.close(1);
  assert.equal(w.win.value.place.path, "B:");
  w.switchTo(2);
  w.close(w.active.value);
  assert.equal(w.win.value.place.path, "B:", "closing the last tab shows the one before");
  w.close(0);
  assert.equal(w.wins.value.length, 2, "home never closes");
});

test("after a connection's folder changes, its tabs go back to its top", () => {
  w.go(folder("/storage/emulated/0/DCIM"));
  w.openTab(folder("Area51:"));
  w.go(folder("Area51:Photos/2026"));
  w.rehome("Area51:");
  assert.deepEqual(places(), ["/storage/emulated/0/DCIM", "Area51:"], "other tabs stay where they are");
});
