// The Connections store, against a fake engine.
import { engine } from "./fake-engine.mjs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { useConnections } from "../src/state/useConnections.ts";

const c = useConnections();
const conn = (name, last_check = null) => ({
  name, scheme: "smb", host: "nas.local", port: null, username: "me", root: "media", options: {},
  encrypted: true, networked: true, rootless: null, clear: null, place: `nas.local › ${name}`, last_check,
});

test("checking every connection asks about each once and reloads once", async () => {
  let listed = 0;
  const tested = [];
  engine.answer = async (cmd, args) => {
    if (cmd === "list_connections") {
      listed++;
      return [conn("a"), conn("b"), conn("c")];
    }
    if (cmd === "test_connection") {
      tested.push(args.name);
      if (args.name === "b") throw "`b` refused the credentials it was given";
      return { entries: 2, root: "media", names: ["x/", "y"], accepts_files: true };
    }
    return null;
  };
  await c.load();
  listed = 0;
  await c.checkAll();
  assert.deepEqual(tested.sort(), ["a", "b", "c"]);
  assert.equal(listed, 1, "one reload for the whole round, not one per connection");
  // What a check found is kept for the names; a failure keeps nothing.
  assert.deepEqual(Object.keys(c.found.value).sort(), ["a", "c"]);
  assert.equal(c.checkingAll(), false);
});

test("a check pressed twice runs once", async () => {
  let tests = 0;
  engine.answer = async (cmd) => {
    if (cmd === "test_connection") {
      tests++;
      await new Promise((r) => setTimeout(r, 5));
      return { entries: 0, root: "media", names: [], accepts_files: true };
    }
    return cmd === "list_connections" ? [conn("a")] : null;
  };
  await Promise.all([c.check("a"), c.check("a")]);
  assert.equal(tests, 1);
});

test("the list survives the engine refusing to give it", async () => {
  engine.answer = async (cmd) => {
    if (cmd === "list_connections") throw "the journal is locked";
    return null;
  };
  await c.load();
  assert.equal(c.problem.value, "the journal is locked");
  assert.equal(c.loaded.value, true);
});
