// The mark a place carries on its tab and rows: home, this phone, a card, or
// a connection in its kind's colour.
import { test } from "node:test";
import assert from "node:assert/strict";

const { CARD_LOOK, PHONE_LOOK, connectionLook, folderLook } = await import("../src/phone/lib/look.ts");
const volumes = [
  { label: "Internal storage", path: "/storage/emulated/0", removable: false },
  { label: "SD card", path: "/storage/1A2B-3C4D", removable: true },
];
const schemes = { Area51: "smb", "Area51-Photos": "ftp" };
const schemeOf = (name) => schemes[name];

test("folders on the phone, a card and connections each get their own mark", () => {
  assert.equal(folderLook("/storage/emulated/0/DCIM", volumes, schemeOf), PHONE_LOOK);
  assert.equal(folderLook("/storage/1A2B-3C4D/Music", volumes, schemeOf), CARD_LOOK);
  assert.deepEqual(folderLook("Area51:Photos", volumes, schemeOf), connectionLook("smb"));
});

test("an FTP and an SMB connection are told apart by colour", () => {
  assert.notEqual(connectionLook("ftp").tint, connectionLook("smb").tint);
  assert.equal(connectionLook("ftps").tint, connectionLook("ftp").tint, "FTP over TLS is still FTP");
  assert.equal(connectionLook(undefined).art, "nas", "a connection not loaded yet still looks like one");
});
