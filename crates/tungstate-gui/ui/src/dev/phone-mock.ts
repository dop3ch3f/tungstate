// A fake engine for phone.html: one NAS with a few folders, so the phone
// screens can be photographed without a phone. `?none` starts with no NAS,
// `?tab=connections` opens a tab, `?press=Media|2026` taps buttons by their text in
// turn, to photograph where they lead.

import { createApp } from "vue";
import { mockIPC } from "@tauri-apps/api/mocks";
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
    default: throw new Error(`phone mock has no answer for ${cmd}`);
  }
});

// Headless Chrome will not make a window narrower than 500px, so the app is
// held to a phone's 390 points here and the shot is cropped to it.
const app = document.getElementById("app")!;
app.style.width = "390px";
app.style.height = "844px";

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
