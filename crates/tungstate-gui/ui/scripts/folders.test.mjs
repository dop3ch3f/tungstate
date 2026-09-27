// The Organize section's state, against a fake engine.
import { engine } from "./fake-engine.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { useFolders } from "../src/state/useFolders.ts";

const later = (ms, value) => new Promise((r) => setTimeout(() => r(value), ms));

test("opening a second folder while the first loads cannot pair it with the first's preview", async () => {
  // Found by audit: `open` set the folder before its busy check, so the first
  // folder's preview arrived under the second folder's name, and Tidy up
  // would then act on a folder whose preview nobody had seen.
  engine.answer = (cmd, args) => later(cmd === "folder_preview" ? 30 : 0, { root: args?.root });
  const f = useFolders();
  const first = f.open("/a");
  await later(5);
  await f.open("/b");
  await first;
  assert.equal(f.root.value, "/a");
  assert.equal(f.preview.value.root, "/a");
});
