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

/** `?bare=1` answers every list with nothing, which is how the empty states
 *  are photographed without emptying the demo folders. */
const bare = new URLSearchParams(location.search).has("bare");

mockIPC((cmd, args) => {
  const a = (args ?? {}) as Record<string, any>;
  if (bare && ["governed", "list_links", "list_connections", "recent", "history", "whereis", "archives", "interrupted"].includes(cmd)) {
    return [];
  }
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
    case "find_duplicates": return found(a.target ?? `${DEMO}/messy-downloads`);
    case "duplicate_action": return null;
    case "stop_finding_duplicates": return null;
    case "clear_duplicates":
      return { files: 3, bytes: 41_943_040, plan: 7, reversible: true, dropped: 1, failed: [] };
    case "list_connections":
      return [{ name: "nas", scheme: "ftp", host: "nas.local", port: 21, username: "me", root: "/volume1/media", options: {}, encrypted: false, networked: true, rootless: null }];
    case "test_connection": return { entries: 14, root: "/volume1/media", names: ["Movies", "Photos"], accepts_files: true };
    case "archives": return [{ name: "2026-09-20T11-02-44", archived_at: Date.now() - 86_400_000, size: 184_320, links: 2, connections: 1, operations: 412 }];
    case "preview_link":
      return { overlapping: [], fresh: 3, same_size: 1, clashes: 1, too_recent: 0, bytes: 629_145_600, removes_originals: true,
        items: listings[`${DEMO}/nas/incoming`].entries.map((e, i) => ({ path: e.path, size: e.size, outcome: ["move", "move", "move", "check", "clash"][i] ?? "move", existing: null, towards: "forward" })) };
    case "interrupted": return [];
    case "places": return [{ label: "Demo", path: DEMO }, { label: "NAS", path: `${DEMO}/nas` }];
    case "last_panes": return { left: `${DEMO}/photos-by-nothing`, right: `${DEMO}/nas/incoming` };
    case "browse": return listings[a.path] ?? { path: a.path, parent: null, entries: [] };
    case "plugin:event|listen": return 1;
    default: return null;
  }
});

/** A scan's answer, built from the fixture's real paths and sizes. */
function found(root: string) {
  const files = listings[`${DEMO}/photos-by-nothing`].entries.slice(0, 3);
  const [one, two, three] = files;
  const copy = (path: string, size: number, mtime: string, extra: Record<string, unknown> = {}) => ({
    path,
    name: path.split("/").pop() ?? path,
    size,
    mtime,
    keep: false,
    alike: null,
    thumb: null,
    ...extra,
  });
  const big = one?.size ?? 1_700_000;
  const mid = two?.size ?? 2_400_000;
  return {
    root,
    files: 192,
    extra_files: 217 + 2 + 1 + 1 + 1,
    reclaimable: 3_400_000_000 + big * 2 + mid + 240_000 + 3_900_000,
    unsure: 1,
    networked: false,
    can_trash: true,
    looked_alike: true,
    rows: [
      {
        id: "f1",
        kind: "folders",
        claim: "identical",
        folder: true,
        reclaimable: 3_400_000_000,
        sure: true,
        files: 214,
        copies: [
          copy("Pictures/Japan 2023", 3_400_000_000, "2023-11-02T09:00:00Z", { keep: true }),
          copy("Desktop/Japan 2023 copy", 3_400_000_000, "2024-04-18T17:40:00Z"),
        ],
      },
      {
        id: "d1",
        kind: "video",
        claim: "identical",
        folder: false,
        reclaimable: big * 2,
        sure: true,
        files: 0,
        copies: [
          copy("holiday-final-2.mp4", big, "2025-08-14T10:02:00Z", { keep: true }),
          copy("clips/IMG_4471.mov", big, "2026-01-20T19:07:00Z"),
          copy("backup/holiday.mov", big, "2026-02-02T09:00:00Z"),
        ],
      },
      {
        id: "d2",
        kind: "documents",
        claim: "identical",
        folder: false,
        reclaimable: mid,
        sure: false,
        files: 0,
        copies: [
          copy("receipts/2026-03.pdf", mid, "2026-03-01T08:00:00Z", { keep: true }),
          copy("Downloads/receipt (1).pdf", mid, "2026-03-11T11:20:00Z"),
        ],
      },
      {
        id: "same-1",
        kind: "pictures",
        claim: "same",
        folder: false,
        reclaimable: 240_000,
        sure: true,
        files: 0,
        copies: [
          copy("Pictures/IMG_4471.jpg", 4_200_000, "2026-02-01T12:00:00Z", { keep: true }),
          copy("Desktop/for the web.jpg", 240_000, "2026-02-03T15:30:00Z", { alike: 100 }),
        ],
      },
      {
        id: "like-1",
        kind: "pictures",
        claim: "similar",
        folder: false,
        reclaimable: 3_900_000,
        sure: true,
        files: 0,
        copies: [
          copy("Pictures/DSC00412.jpg", 4_100_000, "2026-02-01T12:00:01Z", { keep: true }),
          copy("Pictures/DSC00413.jpg", 3_900_000, "2026-02-01T12:00:02Z", { alike: 84 }),
        ],
      },
    ],
    linked: [
      { id: "l1", names: ["clip.mp4", "backup/clip-link.mp4"], size: three?.size ?? 200_000_000 },
    ],
    unchecked: [
      { path: "Downloads/broken.jpg", why: "could not read it: premature end of image" },
    ],
  };
}

const params = new URLSearchParams(location.search);
const scene = params.get("scene") ?? "home";
document.documentElement.dataset.theme = params.get("theme") ?? "retro";

const { createApp } = await import("vue");
const { default: App } = await import("../App.vue");
const { useNav } = await import("../nav");
const { useFolders } = await import("../state/useFolders");
const { useTransfer } = await import("../state/useTransfer");
const { ask } = await import("../ui/useDialog");
const { useDupes } = await import("../state/useDupes");

createApp(App).mount("#app");

const nav = useNav();
const f = useFolders();
const t = useTransfer();
const dz = useDupes();
const DL = `${DEMO}/messy-downloads`;

/** Open a governed folder straight at its preview. */
async function open(root: string) {
  const step = (n: string) => (document.documentElement.dataset.step = n);
  step("go");
  nav.go("folder");
  step("list");
  await f.listRegistered();
  step("open");
  await f.open(root);
  step("tick");
  await tick();
  step("done");
}
/** Click one of the preview's four views by its label. */
function view(label: string) {
  const tabs = Array.from(document.querySelectorAll<HTMLButtonElement>(".views button"));
  tabs.find((b) => b.textContent?.trim().startsWith(label))?.click();
}
function type(sel: string, text: string) {
  const el = document.querySelector<HTMLInputElement>(sel);
  if (!el) return;
  el.value = text;
  el.dispatchEvent(new Event("input"));
}

const tick = () => new Promise((r) => setTimeout(r, 60));
const click = (sel: string) => document.querySelector<HTMLButtonElement>(sel)?.click();
/** Open one of the Transfer window's tabs by position, since its state is local. */
async function tab(n: number) {
  await tick();
  document.querySelectorAll<HTMLButtonElement>(".dh-tabs button")[n]?.click();
  await tick();
}

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
  "drain-run": async () => {
    nav.go("drain");
    await tab(1);
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
  "dupes-start": () => nav.go("dupes"),
  "dupes-scanning": async () => {
    nav.go("dupes");
    await tick();
    dz.phase.value = "scanning";
    dz.root.value = `${DEMO}/photos-by-nothing`;
    dz.progress.value = {
      looked: 4210,
      read: 12,
      recalled: 180,
      bytes: 14_680_064,
      path: `${DEMO}/photos-by-nothing/DSC00412.jpg`,
      stage: "identical",
    };
  },
  "dupes-found": async () => {
    nav.go("dupes");
    await dz.look(`${DEMO}/photos-by-nothing`);
  },
  "dupes-alike": async () => {
    nav.go("dupes");
    await dz.look(`${DEMO}/photos-by-nothing`);
    dz.drawer.value = { claim: "similar", kind: null };
    dz.highlighted.value = null;
  },
  "dupes-rewrapped": async () => {
    nav.go("dupes");
    await dz.look(`${DEMO}/photos-by-nothing`);
    dz.drawer.value = { claim: "same", kind: null };
    dz.opened.value = new Set(["same-1"]);
    dz.highlighted.value = "Desktop/for the web.jpg";
  },
  "dupes-done": async () => {
    nav.go("dupes");
    await dz.look(`${DEMO}/photos-by-nothing`);
    await dz.clear("set-aside");
  },
  settings: () => nav.go("settings"),
  "drain-connections": async () => { nav.go("drain"); await tab(3); },
  "drain-runs": async () => { nav.go("drain"); await tab(1); },
  "drain-add-connection": async () => { nav.go("drain"); await tab(3); click(".cx-top button"); },
  "drain-pairs": async () => { nav.go("drain"); await tab(2); },
  "drain-add-pair": async () => { nav.go("drain"); await tab(2); click(".lk-top button"); },
// --- the states that only appear after something has happened ------------
  "folder-moves": async () => { await open(DL); view("Every move"); },
  "folder-alone": async () => { await open(DL); view("Left alone"); },
  "folder-rules": async () => { await open(DL); view("Rules"); },
  "folder-putback": async () => {
    await open(DL);
    f.putBackCount.value = 28;
    f.preview.value = { ...f.preview.value!, undoable: null };
  },
  "folder-working": async () => { await open(DL); f.busy.value = "Tidying. This does not report progress"; },
  "folder-error": async () => { await open(DL); f.problem.value = "the rules in this folder will not load: line 3, column 9: expected `=`"; },
  "folder-cooldown": async () => {
    await open(DL);
    f.preview.value = { ...f.preview.value!, tidy: true, files: 0 as never, waiting: 4, longest_wait: 96 };
  },
  "folder-already-tidy": async () => {
    await open(DL);
    f.preview.value = { ...f.preview.value!, tidy: true, files: 0 as never, waiting: 0 };
  },
  "folder-unsettled": async () => {
    await open(DL);
    f.preview.value = { ...f.preview.value!, settles: false };
  },
  "folder-broken": async () => {
    nav.go("folder");
    await f.look(DL);
    f.outcomes.value = [
      { name: "(the rules you have)", summary: "", loads: false, settles: true, files: 0, of: 30, bytes: 0, created: 0, removed: 0, example: null },
      ...f.outcomes.value.slice(1),
    ];
  },
  "folder-never-settles": async () => {
    nav.go("folder");
    await f.look(DL);
    f.outcomes.value = f.outcomes.value.map((o, i) => (i === 1 ? { ...o, settles: false } : o));
  },
  "drain-conflict": async () => {
    nav.go("drain");
    t.conflict.value = { path: "holiday.jpg", incoming_size: 2_411_724, existing_size: 1_204_000 };
  },
  "drain-identical": async () => {
    nav.go("drain");
    t.identical.value = { path: "clip-2.mov", size: 209_715_200 };
  },
  "drain-stranded": async () => {
    nav.go("drain");
    await tick();
    t.stranded.value = [{ link: "videos-to-nas", source: `${DEMO}/to-drain`, destination: `${DEMO}/nas/incoming`, files: 2, bytes: 419_430_400, names: ["clip-2.mov", "clip-3.mov"] }];
  },
  "drain-stopping": async () => {
    await scenes["drain-run"]!();
    t.stopping.value = true;
  },
  "drain-done": async () => {
    nav.go("drain");
    await tab(1);
    t.summary.value = {
      transferred: 3, already_present: 1, skipped: 0, quarantined: 1, failed: 1,
      bytes: 629_145_600, recovered: 2, pruned: 0, cancelled: false, destination_lost: false,
      failures: [{ path: "talk.mp4", reason: "the far side refused the write: permission denied" }],
    };
  },
  "drain-deaf": async () => {
    nav.go("drain");
    await tab(1);
    t.deaf.value = "This window cannot hear the engine, so a running transfer will report nothing here. Transfers themselves are unaffected.";
  },
  "history-nothing-found": async () => { nav.go("history"); await tick(); type("input", "nothing like this"); click(".h-find button"); },
  "drain-preview-pair": async () => { nav.go("drain"); await tab(2); await tick(); click(".lk-do button"); },
};
try {
  await scenes[scene]?.();
} catch (e) {
  // A scene that throws must say so: a silent one photographs the wrong screen.
  document.title = `scene failed: ${String(e)}`;
  document.documentElement.dataset.sceneError = String(e);
}
document.documentElement.dataset.ready = "1";
