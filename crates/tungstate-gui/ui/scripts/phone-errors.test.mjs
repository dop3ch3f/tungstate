// What the phone says when a connection fails: one plain sentence, with the
// engine's own text kept for Details.
import { test } from "node:test";
import assert from "node:assert/strict";
import { plainError } from "../src/phone/lib/errors.ts";

const OPENDAL = "Unexpected (persistent) at list, context: { service: ftp, path: / } => 530 Login incorrect.";

test("a refused login says so, whatever the library wrapped round it", () => {
  const p = plainError(OPENDAL, "Area51");
  assert.equal(p.says, "The username or password wasn't accepted.");
  assert.equal(p.detail, OPENDAL);
  assert.equal(plainError("`Area51` refused the credentials it was given").says, "The username or password wasn't accepted.");
});

test("a keychain failure is not mistaken for a wrong password", () => {
  assert.doesNotMatch(plainError("`Area51` was saved, but its password was not: no keyring").says, /wasn't accepted/);
});

test("a folder that won't take files says so before anything about passwords", () => {
  assert.match(plainError("/ lists but refuses files").says, /doesn't take new files/);
  assert.match(plainError("550 Permission denied on STOR").says, /doesn't take new files/);
});

test("network trouble names the place and what to check", () => {
  assert.match(plainError("connection timed out", "Area51").says, /^Area51 didn't answer/);
  assert.match(plainError("Connection refused (os error 61)", "Area51").says, /^Area51 refused/);
  assert.match(plainError("failed to lookup address information", "nas.local").says, /^Couldn't find nas.local/);
});

test("a short sentence nobody needs to decode is shown as it is", () => {
  const p = plainError("this needs a user name; signing in as a guest is not supported");
  assert.equal(p.says, "this needs a user name; signing in as a guest is not supported");
  assert.equal(p.detail, null);
});

test("anything long and unknown is summed up, the original kept", () => {
  const raw = `Unexpected (temporary) at read, context: { service: ftp } => ${"x".repeat(200)}`;
  const p = plainError(raw, "Area51");
  assert.equal(p.says, "Something went wrong talking to Area51.");
  assert.equal(p.detail, raw);
});

test("an Error is read by its message", () => {
  assert.equal(plainError(new Error("No route to host"), "Area51").says.startsWith("Couldn't find Area51"), true);
});
