// A fake engine for phone.html: one NAS with a few folders, so the phone
// screens can be photographed without a phone. `?none` starts with no NAS,
// `?tab=connections` opens a tab, `?press=Media|2026` taps buttons by their text in
// turn, to photograph where they lead. `?run=done|kept|lost|none` says how a Free
// up run ends; without it the run stays going until Pause or Stop.
// `?clear=no|old` has the person refuse removing them, or an Android too old
// to ask; `?clear=wait` leaves the question open. `?android` for its words.
// Files: `?files=ask` before All files access; `?open=PATH` opens a window
// there, `?second=PATH` a second window; `?hold=Name` long-presses a row; `?move=run|done|paused` says where a
// pasted transfer is; `?identical` has it ask about an identical copy.

import { createApp } from "vue";
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import type * as T from "../engine/types";
import "../styles/tokens.css";
import "../styles/base.css";
import PhoneApp from "../phone/PhoneApp.vue";
import { usePhoneNav, type Tab } from "../phone/nav";
import { useWindows } from "../phone/state/useWindows";

const params = new URLSearchParams(location.search);
document.documentElement.dataset.theme = "retro";
document.documentElement.dataset.tone = params.has("dark") ? "dark" : "light";

const area51: T.Connection = {
  name: "Area51", scheme: "smb", host: "area51.local", port: null, username: "me",
  root: "PlexMediaServer", options: {}, encrypted: false, networked: true, rootless: null, clear: null,
  place: "area51.local › PlexMediaServer", last_check: { at: Date.now() - 3_600_000, ok: true, note: "" },
};
let saved: T.Connection[] = params.has("none") ? [] : [area51];
let checked = 0;
let photoAccess: T.PhotoAccess = (params.get("photos") as T.PhotoAccess | null) ?? "full";

const GB = 1024 ** 3;
const dir = (path: string, name: string): T.Entry => ({ name, path: `${path}/${name}`.replace(":/", ":"), is_dir: true, size: 0, modified: null });
const file = (path: string, name: string, size: number): T.Entry => ({ name, path: `${path}/${name}`.replace(":/", ":"), is_dir: false, size, modified: null });
const TREE: Record<string, T.Entry[]> = {
  "Area51:": [dir("Area51:", "Media"), dir("Area51:", "Phone"), dir("Area51:", "Backups")],
  "Area51:Media": [dir("Area51:Media", "2026"), dir("Area51:Media", "2025"), file("Area51:Media", "wedding-final.mov", 18 * GB), file("Area51:Media", "backup-old.zip", 9 * GB), file("Area51:Media", "IMG_4411.HEIC", 3_400_000)],
  "Area51:Phone": [],
};
// The phone's own storage, a little of it.
const SD = "/storage/emulated/0";
const day = (n: number) => Date.UTC(2026, 10, 3, 9, 12) - n * 86_400_000;
const at = (path: string, name: string, size: number, ago: number, isDir = false): T.Entry => ({ name, path: `${path}/${name}`, is_dir: isDir, size, modified: day(ago) });
Object.assign(TREE, {
  [SD]: [at(SD, "DCIM", 0, 1, true), at(SD, "Download", 0, 2, true), at(SD, "Pictures", 0, 9, true), at(SD, "Music", 0, 40, true), at(SD, "Documents", 0, 20, true)],
  [`${SD}/DCIM`]: [at(`${SD}/DCIM`, "Camera", 0, 1, true), at(`${SD}/DCIM`, "Screenshots", 0, 1, true)],
  [`${SD}/DCIM/Camera`]: [
    at(`${SD}/DCIM/Camera`, "IMG_20261102_0912.jpg", 4_400_000, 1), at(`${SD}/DCIM/Camera`, "IMG_20261102_0910.jpg", 4_100_000, 1),
    at(`${SD}/DCIM/Camera`, "VID_20261101_1840.mp4", 222_000_000, 2), at(`${SD}/DCIM/Camera`, "IMG_20261031_1204.jpg", 3_700_000, 3),
    at(`${SD}/DCIM/Camera`, "IMG_20261031_1201.jpg", 4_200_000, 3), at(`${SD}/DCIM/Camera`, "VID_20261030_2112.mp4", 100_000_000, 4),
    at(`${SD}/DCIM/Camera`, "Trip", 0, 5, true),
  ],
});
TREE["Area51:Photos"] = [dir("Area51:Photos", "2026"), dir("Area51:Photos", "2025"), dir("Area51:Photos", "Phone")];
TREE["Area51:"]!.push(dir("Area51:", "Photos"));
const VOLUMES: T.Volume[] = [
  { label: "Internal storage", path: SD, total: 128 * GB, free: 61 * GB, removable: false },
  { label: "SD card", path: "/storage/1A2B-3C4D", total: 64 * GB, free: 22 * GB, removable: true },
];
TREE["/storage/1A2B-3C4D"] = [at("/storage/1A2B-3C4D", "Movies", 0, 30, true)];
const tally = (gb: number, count: number): T.Tally => ({ count, bytes: Math.round(gb * GB) });
let filesAccess: T.PhotoAccess = params.get("files") === "ask" ? "ask" : "full";

/** Findings: a spread over three years, a favourite among them. */
const FINDINGS: T.PhotoAsset[] = [2026, 2025, 2024].flatMap((year, y) =>
  Array.from({ length: [9, 7, 4][y]! }, (_, i) => ({
    id: `content://media/${year}/${i}`, name: `${i % 3 ? "IMG" : "VID"}_${year}07${String(12 - i).padStart(2, "0")}_09${String(i).padStart(2, "0")}.${i % 3 ? "jpg" : "mp4"}`,
    size: i % 3 ? 4_300_000 : 1_800_000_000, takenMs: Date.UTC(year, 6, 12 - i), year, kind: (i % 3 ? "photo" : "video") as T.PhotoAsset["kind"], favourite: year === 2026 && i === 2,
  })));

/** A pasted transfer, played out as the engine's events. */
function playTransfer(req: T.TransferRequest) {
  const names = req.legs.flatMap((l) => l.names);
  const files = names.map((n) => ({ path: n, size: n.endsWith(".mp4") ? 200_000_000 : 4_000_000 }));
  const how = params.get("move") ?? "run";
  const job = { id: 7, name: null, source: req.legs[0]!.source, destination: req.legs[0]!.destination, exchange: false, removes_originals: req.source_policy === "delete", files: files.length, bytes: 0 };
  setTimeout(async () => {
    await emit("transfer://job", job);
    await emit("transfer://planned", files);
    const finish = how === "run" ? Math.ceil(files.length / 2) : how === "paused" ? 1 : files.length;
    for (const f of files.slice(0, finish)) {
      await emit("transfer://started", { path: f.path, size: f.size });
      await emit("transfer://finished", { path: f.path, outcome: "transferred", detail: null });
    }
    if (how === "run" && files[finish]) {
      await emit("transfer://started", { path: files[finish].path, size: files[finish].size });
      await emit("transfer://advanced", { path: files[finish].path, done: files[finish].size / 3, total: files[finish].size });
      if (params.has("identical")) await emit("transfer://identical", { path: files[finish].path, size: files[finish].size });
    }
    if (how !== "run") {
      await emit("transfer://done", { job: 7, transferred: finish, already_present: 0, skipped: 0, quarantined: 0, failed: 0, bytes: 0, recovered: 0, pruned: 0, cancelled: how === "paused", destination_lost: false, failures: [] });
    }
  }, 30);
}

const parentOf = (p: string) => (p.endsWith(":") ? null : p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : p.slice(0, p.indexOf(":") + 1));

mockIPC((cmd, args) => {
  const a = args as Record<string, unknown>;
  switch (cmd) {
    case "list_connections": return saved;
    case "browse": {
      const path = String(a.path);
      return { path, parent: parentOf(path), entries: TREE[path] ?? [] } satisfies T.Listing;
    }
    case "volumes": return VOLUMES;
    case "plugin:photos|files_access": return filesAccess;
    case "plugin:photos|ask_files": filesAccess = "full"; return filesAccess;
    case "plugin:photos|categories":
      return { images: tally(12.4, 4120), videos: tally(31, 212), audio: tally(2.2, 480), documents: tally(0.84, 96), downloads: tally(4.1, 318), apps: tally(9.6, 14) } satisfies T.Categories;
    case "plugin:photos|category_files":
      return (TREE[`${SD}/DCIM/Camera`] ?? []).filter((e) => !e.is_dir).map((e) => ({ path: e.path, name: e.name, size: e.size, modifiedMs: e.modified ?? 0 })) satisfies T.StoredFile[];
    case "plugin:photos|open": case "plugin:photos|share": return null;
    case "free_up_findings": return FINDINGS;
    case "make_folder": return `${String(a.inside)}/${String(a.called)}`;
    case "rename_entry": return String(a.path);
    case "delete_entries": return (a.paths as string[]).length;
    case "transfer_queue": return { running: null, waiting: [] } satisfies T.QueueView;
    case "interrupted": return [];
    case "cancel_run": return null;
    case "resolve_identical": return null;
    case "start_transfer": playTransfer(a.request as T.TransferRequest); return { started: true, waiting: 0, job: 7 } satisfies T.Accepted;
    case "settings_problems": return [];
    case "test_settings": return { entries: 3, root: "PlexMediaServer", names: ["Media", "Phone", "Backups"], accepts_files: true, server: null } satisfies T.Probe;
    case "add_connection": {
      const f = a.form as T.ConnectionForm;
      saved = [...saved, { ...area51, ...f, place: `${f.host} › ${f.root}`, last_check: null } as T.Connection];
      return null;
    }
    // The photo library: `?photos=ask|denied|limited`, full by default.
    case "plugin:photos|access": return photoAccess;
    case "plugin:photos|ask": photoAccess = "full"; return photoAccess;
    case "plugin:photos|summary": {
      const f = a.filter as T.PhotoFilter;
      // Fewer and smaller the narrower the filter, so the screens visibly respond.
      const scale = (f.kind === "videos" ? 0.6 : f.kind === "screenshots" ? 0.03 : 1) * (f.olderThanDays === 0 ? 1.3 : f.olderThanDays >= 365 ? 0.55 : 1);
      const year = (y: number, count: number, gb: number) => ({ year: y, count: Math.round(count * scale), bytes: Math.round(gb * scale * GB) });
      return { years: [year(2026, 1204, 14.2), year(2025, 1688, 19.6), year(2024, 322, 7.4)], favourites: { count: 96, bytes: Math.round(2.1 * GB) } } satisfies T.PhotoSummary;
    }
    // A Free up run, part-way through 41 GB. Late enough that the screen has
    // a pace to work out the time left from.
    case "free_up_start": {
      const ending = params.get("run");
      setTimeout(() => void emit("freeup://progress", { done: 1448, checked: 1448, count: 3214, bytes: Math.round(18.4 * GB), total: 41 * GB, current: "IMG_4471.HEIC" } satisfies T.FreeUpTick), ending ? 50 : 5200);
      const ran = (checked: number, gb: number, extra: Partial<T.FreeUpRan> = {}): T.FreeUpRan => ({ checked, checked_bytes: Math.round(gb * GB), kept: 3214 - checked, stopped: false, problem: null, reason: checked < 3214 ? "IMG_0412.HEIC: the photo library would not open it" : null, ...extra });
      const end = { done: ran(3214, 41), kept: ran(3190, 40.6), lost: ran(0, 0, { problem: "Could not reach Area51: the connection went away part-way, so the run stopped", reason: null }), none: ran(0, 0) }[ending ?? ""];
      if (end) {
        checked = end.checked;
        setTimeout(() => void emit("freeup://done", end), 100);
      }
      return null;
    }
    case "free_up_stop":
      checked = 1448;
      setTimeout(() => void emit("freeup://done", { checked, checked_bytes: Math.round(18.4 * GB), kept: 1766, stopped: true, problem: null, reason: null } satisfies T.FreeUpRan), 50);
      return null;
    case "free_up_clear": {
      const answer = params.get("clear");
      if (answer === "wait") return new Promise(() => {});
      return { removed: answer === "no" || answer === "old" ? 0 : checked, supported: answer !== "old" } satisfies T.Removed;
    }
    default: throw new Error(`phone mock has no answer for ${cmd}`);
  }
}, { shouldMockEvents: true });

// Headless Chrome will not make a window narrower than 500px, so the app is
// held to a phone's 390 points here and the shot is cropped to it.
const app = document.getElementById("app")!;
app.style.width = "390px";
app.style.height = "844px";
// Makes #app the box that `position: fixed` sheets are placed in, as the
// phone's screen is; otherwise they span the 500px window and are cropped.
app.style.transform = "translateZ(0)";

const tab = params.get("tab") as Tab | null;
if (tab) usePhoneNav().go(tab);
const opening = params.get("open");
if (opening) useWindows().go(opening.startsWith("category:") ? { kind: "category", category: opening.slice(9) as T.Category } : { kind: "folder", path: opening });
if (params.get("view") === "grid") useWindows().setView("grid");
// `?second=PATH`: a second window open there, with the first still in front.
const second = params.get("second");
if (second) {
  useWindows().second({ kind: "folder", path: second });
  useWindows().switchTo(0);
}
createApp(PhoneApp).mount("#app");

const settle = () => new Promise((r) => setTimeout(r, 120));
// Waits for the button rather than a fixed time: the NAS list arrives after
// mount, and a tap sent before it exists photographs the wrong screen.
async function find(label: string): Promise<HTMLButtonElement | undefined> {
  for (let i = 0; i < 40; i++) {
    const hit = Array.from(document.querySelectorAll("button")).find((b) => b.textContent?.includes(label));
    if (hit) return hit;
    await settle();
  }
}
// `?hold=Name|Name` long-presses rows, as a finger would to start ticking.
for (const label of (params.get("hold") ?? "").split("|").filter(Boolean)) {
  const hit = await find(label);
  if (!hit) { document.title = `no row "${label}"`; break; }
  // A finger always presses before it lifts; the press is what tells a tap
  // after a long-press apart from the long-press's own lift.
  hit.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
  hit.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
  if (document.querySelector(".check")) hit.click();
  else hit.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
  await settle();
}
for (const label of (params.get("press") ?? "").split("|").filter(Boolean)) {
  const hit = await find(label);
  if (!hit) { document.title = `no button "${label}"`; break; }
  hit.click();
  await settle();
}
await settle();
document.documentElement.dataset.ready = "1";
