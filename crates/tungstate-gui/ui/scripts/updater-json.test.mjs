// The manifest every installed copy reads to find a new version.
import { test } from "node:test";
import assert from "node:assert/strict";
import { manifest } from "./updater-json.mjs";

const files = [
  "Tungstate_macos-arm64.app.tar.gz", "Tungstate_macos-arm64.app.tar.gz.sig",
  "Tungstate_macos-x86_64.app.tar.gz", "Tungstate_macos-x86_64.app.tar.gz.sig",
  "Tungstate_0.1.0-alpha.4_x64-setup.exe", "Tungstate_0.1.0-alpha.4_x64-setup.exe.sig",
  "Tungstate_0.1.0-alpha.4_amd64.AppImage", "Tungstate_0.1.0-alpha.4_amd64.AppImage.sig",
  "Tungstate_0.1.0-alpha.4_amd64.deb", "tungstate-0.1.0-alpha.4-macos-arm64.tar.gz",
];
const base = { signature: (name) => `sig of ${name}\n`, tag: "v0.1.0-alpha.4", notes: "n", date: "d", repo: "o/r" };

test("every platform points at its own signed file in the tagged release", () => {
  const m = manifest({ ...base, files });
  assert.equal(m.version, "0.1.0-alpha.4");
  assert.deepEqual(Object.keys(m.platforms).sort(), ["darwin-aarch64", "darwin-x86_64", "linux-x86_64", "windows-x86_64"]);
  assert.equal(m.platforms["darwin-aarch64"].url, "https://github.com/o/r/releases/download/v0.1.0-alpha.4/Tungstate_macos-arm64.app.tar.gz");
  assert.equal(m.platforms["windows-x86_64"].signature, "sig of Tungstate_0.1.0-alpha.4_x64-setup.exe.sig");
});

test("a platform without a signed update stops the release", () => {
  const unsigned = files.filter((f) => f !== "Tungstate_0.1.0-alpha.4_amd64.AppImage.sig");
  assert.throws(() => manifest({ ...base, files: unsigned }), /linux-x86_64/);
});
