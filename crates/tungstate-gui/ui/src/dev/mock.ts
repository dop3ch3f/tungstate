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

const day = 86_400_000;
const pastRun = (plan: number, name: string, files: number, bytes: number, ago: number, undone = false, undoable = !undone) => ({
  plan, root: `${DEMO}/${name}`, name, applied_at: now - ago, files, bytes, undone, undoable,
});
const pastTidies = [
  pastRun(12, "messy-downloads", 28, 1_048_576_000, 2 * 3_600_000),
  pastRun(9, "real-shape", 4, 125_829_120, day),
  pastRun(6, "auto", 1, 2_516_582, 2 * day, true),
  pastRun(3, "messy-downloads", 17, 402_653_184, 6 * day),
];
const pastCleanups = [
  pastRun(11, "Downloads", 12, 3_435_973_836, 5 * 3_600_000),
  pastRun(8, "Pictures", 3, 42_991_616, 3 * day, false, false),
];

// --- syncs: two real-shaped ones, a pair and a set of four ------------------
const sceneName = new URLSearchParams(location.search).get("scene") ?? "";
const member = (name: string, at: string, remote = false) => ({ name, at, remote });
const syncSettings = {
  on_conflict: "quarantine", on_remove: "set-aside", verify: "hash", cooldown_secs: 30, first_check: "full",
};
const syncList: T.SyncView[] = [
  {
    name: "capcut", direction: "all", exact: true, anchor: null, ...syncSettings,
    // Only the launch scene has anything marked to run at launch, or its
    // sheet would sit over every other scene.
    launch: sceneName === "sync-launch" ? "ask" : sceneName === "sync-following" || sceneName === "sync-held" ? "continuous" : "no",
    members: [member("laptop", "~/Movies/CapCut"), member("nas", "nas:capcut", true)],
  },
  {
    name: "photo-archive", direction: "push", exact: false, anchor: "laptop", ...syncSettings,
    launch: sceneName === "sync-launch" ? "quietly" : "no",
    members: [
      member("laptop", "~/Pictures/Exports"),
      member("nas", "nas:photos", true),
      member("spare", "/Volumes/Spare/photos"),
      member("ftp", "ftp:backup/photos", true),
    ],
  },
];
const memberPreview = (name: string, at: string, over: Partial<T.MemberPreview> = {}): T.MemberPreview => ({
  name, at, arriving: 0, arriving_bytes: 0, leaving: 0, replacing: 0, removing: 0, deleting: 0, renaming: 0, parking: 0, holds: 212, ...over,
});
const syncPreview = (over: Partial<T.SyncPreview> = {}): T.SyncPreview => ({
  sync: "capcut",
  fingerprint: "a1b2c3",
  empty: false,
  reversible: true,
  members: [
    memberPreview("laptop", "~/Movies/CapCut", { arriving: 2, arriving_bytes: 734_003_200, leaving: 3, replacing: 1, holds: 214 }),
    memberPreview("nas", "nas:capcut", { arriving: 3, arriving_bytes: 1_288_490_188, leaving: 2, removing: 4, renaming: 9, holds: 409 }),
  ],
  legs: [
    { from: "laptop", to: "nas", files: 3, parked: 0, bytes: 1_288_490_188, through_here: false },
    { from: "nas", to: "laptop", files: 2, parked: 0, bytes: 734_003_200, through_here: false },
  ],
  read: { files: 9, sampled: 0, no_times: 0, moves: 9, parked: 0 },
  removed: [
    { member: "nas", path: "2025/wedding-rough-cut.mp4", because: "deleted", gone: false },
    { member: "nas", path: "2025/wedding-rough-cut-v2.mp4", because: "deleted", gone: false },
    { member: "nas", path: "tests/colour-test.mov", because: "deleted", gone: false },
    { member: "nas", path: "tests/audio-sync.mov", because: "deleted", gone: false },
  ],
  conflicts: [],
  left_alone: [{ path: "exports/today.mp4", why: "too_recent", members: ["laptop"], existing: null }],
  refusals: [],
  ...over,
});
const conflicted: T.SyncConflict[] = [
  { path: "exports/trailer-final.mp4", versions: [{ member: "laptop", size: 412_000_000, modified: now - 3_600_000 }, { member: "nas", size: 398_000_000, modified: now - 7_200_000 }] },
  { path: "exports/thumbnail.png", versions: [{ member: "laptop", size: 2_400_000, modified: now - 600_000 }, { member: "nas", size: 2_100_000, modified: now - 900_000 }] },
];
const pastSyncs: T.PastSync[] = [
  { plan: 31, sync: "capcut", applied_at: now - 3 * 3_600_000, copied: 12, taken_off: 2, renamed: 0, bytes: 4_294_967_296, undone: false, undoable: true },
  { plan: 27, sync: "photo-archive", applied_at: now - day, copied: 184, taken_off: 0, renamed: 9, bytes: 1_073_741_824, undone: false, undoable: true },
  { plan: 22, sync: "capcut", applied_at: now - 4 * day, copied: 3, taken_off: 0, renamed: 0, bytes: 734_003_200, undone: true, undoable: false },
];

mockIPC((cmd, args) => {
  const a = (args ?? {}) as Record<string, any>;
  if (bare && ["governed", "list_links", "list_connections", "recent", "history", "whereis", "archives", "interrupted", "past_tidies", "past_cleanups", "recent_scans", "list_syncs", "past_syncs"].includes(cmd)) {
    return [];
  }
  // A call that never answers, to photograph a button while it waits.
  if (sceneName === "links-busy" && cmd === "run_link") return new Promise(() => {});
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
    case "past_tidies": return pastTidies;
    case "past_cleanups": return pastCleanups;
    case "recent_scans": return [`${DEMO}/messy-downloads`, `${DEMO}/real-shape-media`];
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
    case "list_syncs": return syncList;
    case "following_state":
      if (sceneName === "sync-following" || sceneName === "sync-held") {
        return {
          members: [
            { sync: "capcut", member: "laptop", state: "watching", every_secs: null, why: null },
            { sync: "capcut", member: "nas", state: sceneName === "sync-held" ? "polling" : "paused", every_secs: 120, why: sceneName === "sync-held" ? null : "no route to host" },
          ],
          held: sceneName === "sync-held" ? ["capcut"] : [],
        };
      }
      return { members: [], held: [] };
    case "past_syncs": return pastSyncs;
    case "preview_sync":
      if (a.name === "photo-archive") {
        return syncPreview({
          sync: "photo-archive",
          members: [
            memberPreview("laptop", "~/Pictures/Exports", { leaving: 18 }),
            memberPreview("nas", "nas:photos", { arriving: 6, arriving_bytes: 48_000_000 }),
            memberPreview("spare", "/Volumes/Spare/photos", { arriving: 6, arriving_bytes: 48_000_000 }),
            memberPreview("ftp", "ftp:backup/photos", { arriving: 6, arriving_bytes: 48_000_000 }),
          ],
          removed: [],
          left_alone: [],
        });
      }
      return syncPreview();
    case "make_sync": return syncList[0];
    case "change_sync": return syncList[0];
    case "run_sync": return { started: true, waiting: 0 };
    case "put_back_sync": return { plan: 31, put_back: 2, taken_off: 5, parked_left: 0, revived: [{ member: "laptop", path: "2025/wedding-rough-cut.mp4" }] };
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
const { useSync } = await import("../state/useSync");

createApp(App).mount("#app");

const nav = useNav();
const f = useFolders();
const t = useTransfer();
const dz = useDupes();
const sy = useSync();
/** Open a sync's page and put a preview on it, as a scene wants. */
async function syncAt(index: number, preview?: T.SyncPreview) {
  nav.go("sync");
  await sy.load();
  sy.open(syncList[index]!);
  if (preview) {
    sy.preview.value = preview;
    sy.seen.value = preview.conflicts;
    sy.phase.value = "preview";
  }
}
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
  "sync-start": async () => { nav.go("sync"); await sy.load(); },
  "sync-make": async () => { nav.go("sync"); await sy.load(); sy.making(); },
  "sync-one": () => syncAt(0),
  "sync-four": () => syncAt(1),
  "sync-preview": () => syncAt(0, syncPreview()),
  "sync-nothing": () => syncAt(0, syncPreview({ empty: true, legs: [], removed: [], left_alone: [], read: { files: 0, sampled: 0, no_times: 0, moves: 0, parked: 0 }, members: [memberPreview("laptop", "~/Movies/CapCut"), memberPreview("nas", "nas:capcut")] })),
  "sync-conflict": async () => {
    await syncAt(0, syncPreview({ conflicts: conflicted, removed: [], left_alone: conflicted.map((c) => ({ path: c.path, why: "conflict", members: ["laptop", "nas"], existing: null })) }));
    sy.choices.value = { "exports/thumbnail.png": "laptop" };
  },
  "sync-refused": () =>
    // An unplugged share: nas lists nothing, so an exact sync would carry
    // "everything deleted" to the laptop. Every number agrees with that.
    syncAt(0, syncPreview({
      members: [
        memberPreview("laptop", "~/Movies/CapCut", { removing: 212, holds: 212 }),
        memberPreview("nas", "/Volumes/nas/capcut", { holds: 0 }),
      ],
      legs: [],
      left_alone: [],
      read: { files: 0, sampled: 0, no_times: 0, moves: 0, parked: 0 },
      refusals: [
        { kind: "hollow", member: "nas", held: 212 },
        { kind: "blast", member: "laptop", taking_off: 212, of: 212 },
      ],
      removed: Array.from({ length: 212 }, (_, i) => ({
        member: "laptop", path: `exports/clip-${String(i + 1).padStart(3, "0")}.mp4`, because: "deleted", gone: false,
      })),
    })),
  "sync-deletes": () =>
    syncAt(0, syncPreview({
      reversible: false,
      members: [memberPreview("laptop", "~/Movies/CapCut"), memberPreview("nas", "nas:capcut", { removing: 4, deleting: 4 })],
      removed: syncPreview().removed.map((r) => ({ ...r, gone: true })),
    })),
  "sync-running": async () => {
    await syncAt(0);
    sy.phase.value = "running";
    sy.leg.value = { index: 0, from: "laptop", to: "nas" };
    sy.rows.value = [
      { leg: 0, path: "exports/trailer-final.mp4", size: 412_000_000, state: "live", detail: null, done: 180_000_000 },
      { leg: 0, path: "exports/teaser.mp4", size: 88_000_000, state: "transferred", detail: null, done: 88_000_000 },
      { leg: 0, path: "exports/bts.mp4", size: 790_000_000, state: "waiting", detail: null, done: 0 },
    ];
  },
  "sync-done": async () => {
    await syncAt(0);
    sy.phase.value = "done";
    sy.rows.value = [
      { leg: 0, path: "exports/trailer-final.mp4", size: 412_000_000, state: "transferred", detail: null, done: 412_000_000 },
      { leg: 1, path: "2024/wedding.mp4", size: 734_003_200, state: "transferred", detail: null, done: 734_003_200 },
    ];
    sy.ran.value = {
      sync: "capcut", plan: 31, stopped: false, reversible: true, taken_off: 4, renamed: 9,
      legs: [{ from: "laptop", to: "nas", copied: 1, bytes: 412_000_000, failed: 0 }, { from: "nas", to: "laptop", copied: 1, bytes: 734_003_200, failed: 0 }],
      missed: [{ member: "nas", path: "tests/audio-sync.mov", why: "it has been written to since the run was decided" }],
    };
  },
  "sync-put-back": async () => {
    await syncAt(0);
    await sy.putBack("capcut", 31);
  },
  "sync-settings": async () => { await syncAt(0); await tick(); document.querySelectorAll<HTMLButtonElement>(".so-acts button")[1]?.click(); },
  "sync-done-busy": async () => {
    await scenes["sync-done"]();
    sy.busy.value = 31;
  },
  "links-busy": async () => { nav.go("drain"); await tab(2); await tick(); document.querySelectorAll<HTMLButtonElement>(".lk-do button")[1]?.click(); },
  "sync-launch": () => {},
  "sync-following": () => syncAt(0),
  "sync-held": () => syncAt(0),
  "sync-held-list": async () => { nav.go("sync"); await sy.load(); },
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
