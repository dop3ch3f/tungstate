// What a found server fills in on the add-connection form.
import { test } from "node:test";
import assert from "node:assert/strict";
import { kindsOf, nameFor, nameForFolder, offersLine, portFor, unsaved, upsert } from "../src/phone/lib/discovery.ts";

const area51 = { address: "192.168.31.71", name: "Area51", host: "Area51.local", offers: [{ kind: "ftp", port: 21 }, { kind: "smb", port: 445 }] };
const nameless = { address: "192.168.31.40", name: null, host: null, offers: [{ kind: "smb", port: 445 }] };

test("SMB is suggested before FTP, and both are named", () => {
  assert.deepEqual(kindsOf(area51), ["smb", "ftp"]);
  assert.equal(offersLine(area51), "SMB and FTP");
  assert.equal(offersLine(nameless), "SMB");
});

test("a usual port is left blank, an unusual one filled in", () => {
  assert.equal(portFor(area51, "smb"), null);
  assert.equal(portFor({ ...area51, offers: [{ kind: "ftp", port: 2121 }] }, "ftp"), 2121);
});

test("the name fits the name rule and never repeats one", () => {
  assert.equal(nameFor(area51, []), "Area51");
  assert.equal(nameFor({ ...area51, name: "My Cloud EX2 (home)" }, []), "My-Cloud-EX2-home");
  assert.equal(nameFor(nameless, []), "NAS-40");
  assert.equal(nameFor(area51, ["area51", "Area51-2"]), "Area51-3");
});

test("a server already saved, by address or .local name, is not offered again", () => {
  assert.deepEqual(unsaved([area51, nameless], [{ host: "area51.local" }]), [nameless]);
  assert.deepEqual(unsaved([area51, nameless], [{ host: "192.168.31.40" }]), [area51]);
  assert.deepEqual(unsaved([area51], [{ host: null }]), [area51]);
});

test("a second sighting replaces the first, named servers listed first", () => {
  const list = upsert(upsert([], nameless), { ...area51, offers: [area51.offers[1]] });
  assert.deepEqual(list.map((f) => f.address), ["192.168.31.71", "192.168.31.40"]);
  const more = upsert(list, area51);
  assert.equal(more.length, 2);
  assert.equal(more[0].offers.length, 2);
});

test("each folder on one server gets a name of its own", () => {
  assert.equal(nameForFolder("Area51", "/home/me/Photos", []), "Area51-Photos");
  assert.equal(nameForFolder("Area51", "/home/me/Photos/", ["Area51"]), "Area51-Photos");
  assert.equal(nameForFolder("Area51", "/Old photos (2019)", []), "Area51-Old-photos-2019");
  assert.equal(nameForFolder("Area51", "/", []), "Area51", "the server's top adds nothing");
  assert.equal(nameForFolder("Area51", "/Photos", ["area51-photos"]), "Area51-Photos-2");
});
