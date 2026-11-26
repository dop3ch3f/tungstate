// What each file's line says under a phone Copy or Move, and how its mark and
// bar are chosen. Pure, so these run without a screen.
import { test } from "node:test";
import assert from "node:assert/strict";

const { isMoving, markOf, percentOf, rowWord } = await import("../src/phone/lib/progress.ts");
const row = (state, extra = {}) => ({ path: "a.jpg", size: 200, state, done: 0, checked: 0, ...extra });

test("each engine state reads as a plain word, and a Move says moved", () => {
  assert.equal(rowWord(row("waiting"), "copy"), "Waiting");
  assert.equal(rowWord(row("live"), "copy"), "Sending");
  assert.equal(rowWord(row("checking"), "copy"), "Checking it arrived");
  assert.equal(rowWord(row("transferred"), "copy"), "Copied and checked");
  assert.equal(rowWord(row("transferred"), "move"), "Moved and checked");
  assert.equal(rowWord(row("already-present"), "copy"), "Already there");
  assert.equal(rowWord(row("failed"), "copy"), "Failed");
});

test("a state the engine adds later still gets a known mark", () => {
  assert.equal(markOf("something-new"), "same");
  assert.equal(markOf("failed"), "failed");
  assert.equal(markOf("transferred"), "done");
});

test("only a file on its way gets a bar", () => {
  assert.equal(isMoving("live"), true);
  assert.equal(isMoving("checking"), true);
  assert.equal(isMoving("waiting"), false);
  assert.equal(isMoving("transferred"), false);
});

test("sending counts bytes sent, checking counts bytes read back", () => {
  assert.equal(percentOf(row("live", { done: 50 })), 25);
  assert.equal(percentOf(row("checking", { done: 200, checked: 150 })), 75);
  assert.equal(percentOf(row("live", { done: 500 })), 100, "never past the end");
  assert.equal(percentOf(row("live", { size: 0 })), 0, "an empty file is not a division by nothing");
});
