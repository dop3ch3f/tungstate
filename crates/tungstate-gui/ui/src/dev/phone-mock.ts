// A fake engine for phone.html: one NAS with a few folders, so the phone
// screens can be photographed without a phone. `?none` starts with no NAS,
// `?tab=connections` opens a tab, `?press=Media|2026` taps buttons by their text in
// turn, to photograph where they lead. `?run=done|kept|lost|none` says how a Free
// up run ends; without it the run stays going until Pause or Stop.
// `?clear=no|old` has the person refuse removing them, or an Android too old
// to ask; `?clear=wait` leaves the question open. `?android` for its words.

import { createApp } from "vue";
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import type * as T from "../engine/types";
import "../styles/tokens.css";
import "../styles/base.css";
import PhoneApp from "../phone/PhoneApp.vue";
import { usePhoneNav, type Tab } from "../phone/nav";

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
const parentOf = (p: string) => (p.endsWith(":") ? null : p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : p.slice(0, p.indexOf(":") + 1));

mockIPC((cmd, args) => {
  const a = args as Record<string, unknown>;
  switch (cmd) {
    case "list_connections": return saved;
    case "browse": {
      const path = String(a.path);
      return { path, parent: parentOf(path), entries: TREE[path] ?? [] } satisfies T.Listing;
    }
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
for (const label of (params.get("press") ?? "").split("|").filter(Boolean)) {
  const hit = await find(label);
  if (!hit) { document.title = `no button "${label}"`; break; }
  hit.click();
  await settle();
}
await settle();
document.documentElement.dataset.ready = "1";
