// What Organise shows before anything moves, and which layout it suggests.
import { test } from "node:test";
import assert from "node:assert/strict";
import { destinations, layoutFor, placesUnder } from "../src/phone/lib/organise.ts";

test("moves are grouped by the folder they land in, biggest first", () => {
  const moves = [
    { to: "2026/Documents/pdf/a.pdf" },
    { to: "2026/Documents/pdf/b.pdf" },
    { to: "2026/WhatsApp/Images/jpg/c.jpg" },
    { to: "top.txt" },
  ];
  assert.deepEqual(destinations(moves), [
    { folder: "2026 › Documents › pdf", files: 2 },
    { folder: "2026 › WhatsApp › Images › jpg", files: 1 },
    { folder: "The folder itself", files: 1 },
  ]);
});

test("a folder's name suggests its layout", () => {
  assert.equal(layoutFor("/storage/emulated/0/Download"), "downloads");
  assert.equal(layoutFor("/storage/emulated/0/DCIM/"), "photos");
  assert.equal(layoutFor("/storage/emulated/0/Stuff"), "by-type");
});

test("the whole phone is offered first for duplicates, Download first to organise", () => {
  const root = "/storage/emulated/0";
  assert.equal(placesUnder(root, "dupes")[0].path, root);
  assert.equal(placesUnder(`${root}/`, "organise")[0].path, `${root}/Download`);
});
