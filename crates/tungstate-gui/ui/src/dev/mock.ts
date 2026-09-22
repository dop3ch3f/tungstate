// A fake engine for mock.html, so any screen can be photographed in a browser.
//
// Every answer comes from fixture.json, which scripts/capture-fixture.py writes
// from the demo folders and the real CLI. `?scene=` puts the module-level state
// straight into the screen being checked, because clicking there is exactly
// what this exists to avoid. Only mock.html imports this file.

import { mockIPC } from "@tauri-apps/api/mocks";
import fx from "./fixture.json";
import type * as T from "../engine/types";
import "../styles/tokens.css";
import "../styles/base.css";

const DEMO = Object.keys(fx.listings)[0].replace(/\/to-drain$/, "");
const SUMMARY: Record<string, string> = {
  downloads: "Sort by what each file is",
  photos: "File photos by date",
  documents: "Group documents by kind, then year",
  media: "Year, app, kind, extension, size",
  "by-date": "Everything by year, then month",
  "by-source": "One directory per app it came from",
  "by-type": "One directory per file extension",
};

// `folder compare` prints no byte totals, so bytes are scaled from the
// preview's real total by file count. The only figure here that is derived.
function outcomes(name: keyof typeof fx.compare, total: number): T.Outcome[] {
  return fx.compare[name].map((o) => ({
    ...o,
    name: o.name === "(the rules you have)" ? "(the rules you have)" : o.name,
    summary: SUMMARY[o.name] ?? "",
    bytes: Math.round((total * o.files) / Math.max(o.of, 1)),
  }));
}

const now = fx.now;
const ops: T.Op[] = fx.preview["messy-downloads"].moves.slice(0, 14).map((m, i) => ({
  id: 900 - i,
  status: i === 3 ? "failed" : "done",
  kind: i % 5 === 4 ? "transfer" : "rename",
  source: `${DEMO}/messy-downloads/${m.from}`,
  destination: `${DEMO}/messy-downloads/${m.to}`,
  size: 1_000_000 * (i + 3),
  hash: null,
  link: i % 5 === 4 ? "videos-to-nas" : null,
  note: i === 3 ? "destination refused the write" : null,
  started_at: now - i * 3600,
}));

const learned: T.Learned = {
  levels: ["name"],
  explains: 2,
  of: 30,
  loose: 28,
  as_is: "[[rule]]\nname = \"Images\"\nmatch = [\"Images/**\", \"**/Images/**\"]\npath = \"Images\"\n",
  improved: null,
  suggestions: [
    { headline: "Give the loose files a home.", why: "28 of 30 file(s) sit at the top of the folder, outside the shape." },
    {
      headline: "File the Screenshots files by their names.",
      why: "1 unfiled file(s) are named the way Screenshots names them (`^(Screenshot|Screen Shot|Bildschirmfoto)[ _-]`), which says where they belong without any directory to read.",
    },
  ],
};

const governed: T.FolderView[] = [
  { name: "messy-downloads", root: `${DEMO}/messy-downloads`, has_rules: true, broken: null },
  { name: "real-shape", root: `${DEMO}/real-shape-media`, has_rules: true, broken: null },
  { name: "broken-rules", root: `${DEMO}/broken-rules`, has_rules: true, broken: "line 3, column 9: expected `=`" },
];

const links: T.Link[] = [
  {
    name: "videos-to-nas", source: `${DEMO}/to-drain`, destination: `${DEMO}/nas/incoming`,
    source_policy: "delete", verify: "hash", order: "largest-first", on_conflict: "quarantine", cooldown_secs: 60,
  },
];

const listings = fx.listings as Record<string, T.Listing>;
const previews = fx.preview as Record<string, T.PreviewView>;
const byRoot = (root: string) => previews[root.split("/").pop()!] ?? previews["messy-downloads"];

mockIPC((cmd, args) => {
  const a = (args ?? {}) as Record<string, any>;
  switch (cmd) {
    case "governed": return governed;
    case "learn_folder":
      return a.root.endsWith("real-shape")
        ? { levels: ["year", "name", "kind", "extension", "size"], explains: 192, of: 192, loose: 0, as_is: "", improved: null, suggestions: [] }
        : learned;
    case "compare_folder":
      return a.root.endsWith("real-shape")
        ? outcomes("real-shape", 0)
        : outcomes("messy-downloads", fx.preview["messy-downloads"].bytes);
    case "folder_preview": return byRoot(a.root);
    case "rules_text": return learned.as_is;
    case "recent": case "history": case "whereis": return ops;
    case "list_links": return links;
    case "list_connections": return [];
    case "interrupted": return [];
    case "places": return [{ label: "Demo", path: DEMO }, { label: "NAS", path: `${DEMO}/nas` }];
    case "last_panes": return { left: `${DEMO}/photos-by-nothing`, right: `${DEMO}/nas/incoming` };
    case "browse": return listings[a.path] ?? { path: a.path, parent: null, entries: [] };
    case "plugin:event|listen": return 1;
    default: return null;
  }
});

const params = new URLSearchParams(location.search);
const scene = params.get("scene") ?? "home";

const { createApp } = await import("vue");
const { default: App } = await import("../App.vue");
const { useNav } = await import("../nav");
const { useFolders } = await import("../state/useFolders");
const { useTransfer } = await import("../state/useTransfer");
const { ask } = await import("../ui/useDialog");

createApp(App).mount("#app");

const nav = useNav();
const f = useFolders();
const t = useTransfer();
const DL = `${DEMO}/messy-downloads`;

const scenes: Record<string, () => unknown> = {
  home: () => {},
  "folder-start": async () => { nav.go("folder"); await f.listRegistered(); },
  "folder-read": async () => { nav.go("folder"); await f.look(DL); },
  "folder-read-real": async () => { nav.go("folder"); await f.look(`${DEMO}/real-shape`); },
  "folder-picked": async () => {
    nav.go("folder");
    await f.look(`${DEMO}/real-shape`);
    await new Promise((r) => setTimeout(r, 50));
    document.querySelectorAll<HTMLButtonElement>(".way")[3]?.click();
  },
  "folder-preview": async () => { nav.go("folder"); await f.listRegistered(); await f.open(DL); },
  "folder-tidied": async () => {
    nav.go("folder"); await f.listRegistered(); await f.open(DL);
    f.tidied.value = { moved: 28, skipped: 1, failed: 0, already_tidy: false };
    f.preview.value = { ...f.preview.value!, undoable: 41 };
  },
  "folder-confirm": async () => {
    nav.go("folder"); await f.listRegistered(); await f.open(DL);
    void ask({
      title: "Tidy downloads?",
      why: "This is a large change. Everything it does can be put back, and the button to do that stays on this screen.",
      detail: ["28 files would move, of 30.", "1.0 GB in all.", "6 directories made, 1 emptied."],
      choices: [{ id: "no", label: "Not now" }, { id: "yes", label: "Tidy up", look: "primary" }],
    });
  },
  drain: () => nav.go("drain"),
  "drain-run": () => {
    nav.go("drain");
    const files = listings[`${DEMO}/nas/incoming`].entries.slice(0, 4);
    t.shape.value = { removes_originals: true, at_once: 2 };
    t.atOnce.value = 2;
    t.live.value = files[0]?.path ?? null;
    t.rows.value = files.map((e, i) => ({
      path: e.path, size: e.size, detail: null,
      state: ["live", "checking", "waiting"][i] ?? "waiting",
      done: i === 0 ? Math.round(e.size * 0.62) : i === 1 ? e.size : 0,
      checked: i === 1 ? Math.round(e.size * 0.3) : 0,
    }));
  },
  history: () => nav.go("history"),
};
await scenes[scene]?.();
document.documentElement.dataset.ready = "1";
