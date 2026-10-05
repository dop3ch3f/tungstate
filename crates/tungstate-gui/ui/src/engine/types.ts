// Every shape that crosses the seam, typed once.
//
// Data only: no Vue, no Tauri, no formatting. A screen that needs to know what
// a `PreviewView` is should not have to pull `@tauri-apps/api` into its module
// graph to find out.
//
// These mirror `crates/tungstate-gui/src/{main,govern}.rs` and
// `crates/tungstate-core/src/compare.rs`. `docs/SEAM.md` is the contract they
// implement. Nothing here may be invented: if a field is not in the Rust, the
// window cannot have it.

// --- folders -------------------------------------------------------------

export interface FolderView {
  name: string;
  root: string;
  has_rules: boolean;
  /** The policy's parse error, with line and column, or null. Loaded for the
   *  list rather than on opening, so broken rules are visible before you
   *  click into them. */
  broken: string | null;
}

export interface LayoutView {
  name: string;
  summary: string;
  detail: string;
}

export interface TreeEntry {
  path: string;
  is_dir: boolean;
  size: number;
  /** Whether this entry is one of the ones that move. Both trees mark the
   *  same files, so the eye can follow one across. */
  moves: boolean;
}

export interface MoveView {
  from: string;
  to: string;
  why: string;
}

/**
 * One file the plan did not touch, and the engine's own sentence for why.
 *
 * The sentence is all there is. `plan::Reason` has eight variants and
 * `govern.rs:255` flattens them into prose before they cross, so the window
 * cannot group or count by cause without guessing at wording. It does not
 * guess: it prints what the engine wrote, one file per line. See
 * `docs/SEAM.md` §"What a redesign must keep", property 4.
 */
export interface LeftAloneView {
  path: string;
  why: string;
}

/** Everything one governed folder's preview screen needs. */
export interface PreviewView {
  folder: string;
  before: TreeEntry[];
  after: TreeEntry[];
  moves: MoveView[];
  left_alone: LeftAloneView[];
  /** Files that would move. Not the number of operations. */
  files: number;
  of: number;
  bytes: number;
  /** Past the blast-radius limit. A warning before the button, never a refusal
   *  after it. */
  large: boolean;
  /** False when the rules would keep moving the same files for ever. The one
   *  refusal the window keeps. */
  settles: boolean;
  waiting: number;
  longest_wait: number;
  /** Whether there is anything at all to do. Distinct from `files: 0`. */
  tidy: boolean;
  /** The reorganisation that can be put back, if there is one. */
  undoable: number | null;
}

/**
 * What a tidy did, as numbers the window words for itself.
 *
 * `already_tidy` is a flag and not `moved: 0`, because "there was nothing to
 * do" and "it moved nothing" are different sentences and only the window knows
 * which one it wants to say.
 */
export interface TidyDone {
  moved: number;
  /** Left where they were because they changed while we looked. */
  skipped: number;
  failed: number;
  already_tidy: boolean;
}

export interface PutBackDone {
  files: number;
}

/** A folder's own shape, read back out of it. */
export interface Learned {
  /** One word per level, outermost first: year, name, kind, extension, size. */
  levels: string[];
  /** Files the shape accounts for. */
  explains: number;
  /** Files looked at. Note this is not `Outcome.of`: the probe applies its own
   *  ignore list and the comparison counts every entry it walked, so the two
   *  differ by the `.DS_Store` and the like. A screen must not show both. */
  of: number;
  /** Files sitting loose at the top. */
  loose: number;
  /** Rules that keep the shape it already has. */
  as_is: string;
  /** The same rules with every suggestion applied, when there are any. */
  improved: string | null;
  suggestions: { headline: string; why: string }[];
}

/** What one way of filing would do to this folder. */
export interface Outcome {
  /** The layout's name, or a label for rules that are not a layout. */
  name: string;
  /** Empty for rules that are not a layout, so the window supplies the words. */
  summary: string;
  loads: boolean;
  settles: boolean;
  files: number;
  of: number;
  bytes: number;
  created: number;
  /** Directories it would remove: the number that says a layout is replacing a
   *  shape rather than adding to one. */
  removed: number;
  /** One real move, because a count does not tell you whether you would like
   *  the answer. */
  example: { from: string; to: string } | null;
}

// --- duplicates ----------------------------------------------------------

/** One copy of some content. */
export interface DupeCopy {
  path: string;
  size: number;
  mtime: string | null;
}

/** Which drawer of the window a group belongs in. */
export type DupeKind =
  | "pictures"
  | "video"
  | "sound"
  | "documents"
  | "archives"
  | "applications"
  | "folders"
  | "other";

/** How strong the claim about a group is. `identical` is proof; the other two
 *  are arithmetic on pixels or on sound, and are suggestions. */
export type Claim = "identical" | "same" | "similar";

/** One copy, as a row. `keep` marks the one the pass would keep, which is the
 *  one that starts unticked. `alike` is absent when the match is proof. */
export interface DupeCopy {
  path: string;
  name: string;
  size: number;
  mtime: string | null;
  keep: boolean;
  alike: number | null;
  /** The name of its small picture, served over the `thumb` scheme. */
  thumb: string | null;
}

/** One group of copies, whichever pass found it. `id` is stable across a
 *  rescan, which is what lets a selection survive one. */
export interface DupeRow {
  id: string;
  kind: DupeKind;
  claim: Claim;
  folder: boolean;
  reclaimable: number;
  /** False when the group was matched on samples: shown, never acted on
   *  without confirming first. */
  sure: boolean;
  /** How many files one of the directories holds. Zero for a file group. */
  files: number;
  copies: DupeCopy[];
}

/** Two or more names for one file: a hard link, and not a saving. */
export interface DupeLinked {
  id: string;
  names: string[];
  size: number;
}

/** A file that could not be looked at, and why. Counted rather than skipped. */
export interface Unchecked {
  path: string;
  why: string;
}

export interface Found {
  root: string;
  rows: DupeRow[];
  linked: DupeLinked[];
  unchecked: Unchecked[];
  files: number;
  extra_files: number;
  reclaimable: number;
  unsure: number;
  /** The bytes are a network away, so the groups were sampled. */
  networked: boolean;
  /** Whether the desktop's trash can be offered here. */
  can_trash: boolean;
  /** Whether resemblances were looked for at all. */
  looked_alike: boolean;
}

export interface ScanProgress {
  looked: number;
  read: number;
  recalled: number;
  bytes: number;
  path: string;
  /** Which pass is running: `identical` or `alike`. */
  stage: string;
}

export interface Cleared {
  files: number;
  bytes: number;
  plan: number;
  reversible: boolean;
  /** Groups the confirmation found were not identical after all. */
  dropped: number;
  failed: string[];
}

/** One thing the watcher did or saw. `kind` is `tidied`, `waiting`,
 *  `trouble` or `settled`. */
export interface Notice {
  kind: string;
  folder: string;
  files: number;
  /** The plan, so it can be undone. Zero when there is nothing to undo. */
  plan: number;
  why: string;
  /** Milliseconds since the epoch. */
  at: number;
}

export interface WatchState {
  on: boolean;
  running: boolean;
  /** Folders watched for events. */
  watching: number;
  /** Folders only checked on the hour, because they are on a share. */
  sweeping: number;
  /** Newest first. */
  recent: Notice[];
}

// --- history -------------------------------------------------------------

/** A past tidy or duplicate clean-up, for a section's own history. */
export interface PastRun {
  /** What Put back takes. */
  plan: number;
  root: string;
  /** The folder's name on screen. */
  name: string;
  applied_at: number;
  /** Files moved or set aside; never directories or failures. */
  files: number;
  bytes: number;
  undone: boolean;
  /** False once put back, and for anything sent to the trash. */
  undoable: boolean;
}

export interface Op {
  id: number;
  status: string;
  kind: string;
  source: string | null;
  destination: string | null;
  size: number | null;
  hash: string | null;
  link: string | null;
  note: string | null;
  started_at: number;
}

// --- transfers -----------------------------------------------------------

export interface Link {
  name: string;
  source: string;
  destination: string;
  source_policy: string;
  verify: string;
  order: string;
  on_conflict: string;
  cooldown_secs: number;
}

export type NewLink = Omit<Link, never>;

export interface Entry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: number | null;
}

export interface Listing {
  path: string;
  parent: string | null;
  entries: Entry[];
}

export interface Place {
  label: string;
  path: string;
}

export interface Leg {
  source: string;
  destination: string;
  names: string[];
}

export interface TransferRequest {
  legs: Leg[];
  source_policy: string;
  verify: string;
  on_conflict: string;
  order?: string;
  cooldown_secs?: number;
  save_as: string | null;
}

export interface Prospect {
  path: string;
  size: number;
  outcome: "move" | "check" | "clash" | "hold";
  existing: number | null;
  towards: "forward" | "back";
}

export interface Preview {
  overlapping: string[];
  fresh: number;
  same_size: number;
  clashes: number;
  too_recent: number;
  bytes: number;
  removes_originals: boolean;
  items: Prospect[];
}

/** `started` is false when the files joined a run already in flight, and the
 *  window must not clear the progress it is showing. */
export interface Accepted {
  started: boolean;
  /** Transfers waiting behind the running one, this one included. */
  waiting: number;
  /** This transfer, as the queue and its events name it. */
  job: number;
}

/** A transfer in the queue. `files` and `bytes` are null until counted. */
export interface JobView {
  id: number;
  /** The saved pair's name; a one-off transfer from the browser has none. */
  name: string | null;
  source: string;
  destination: string;
  exchange: boolean;
  removes_originals: boolean;
  files: number | null;
  bytes: number | null;
}

export interface QueueView {
  running: JobView | null;
  waiting: JobView[];
}

export type JobDone = Summary & { job: number };
export interface JobFailed { job: number; message: string }

export interface Failure {
  path: string;
  reason: string;
}

export interface Summary {
  transferred: number;
  already_present: number;
  skipped: number;
  quarantined: number;
  failed: number;
  bytes: number;
  recovered: number;
  pruned: number;
  cancelled: boolean;
  destination_lost: boolean;
  failures: Failure[];
}

export interface Began {
  removes_originals: boolean;
  at_once: number;
}
export interface PlannedFile { path: string; size: number }
export interface Advanced { path: string; done: number; total: number }
export interface Started { path: string; size: number }
export interface Finished { path: string; outcome: string; detail: string | null }
export interface ConflictAsk { path: string; incoming_size: number; existing_size: number }
export interface IdenticalAsk { path: string; size: number }

export interface InterruptedRun {
  link: string;
  source: string;
  destination: string;
  files: number;
  bytes: number;
  names: string[];
}

// --- connections ---------------------------------------------------------

/** `encrypted`, `networked` and `rootless` arrive already decided. The rules
 *  live on `Scheme` in Rust, and re-deriving them here would be a second
 *  implementation of a warning that has to be right. */
export interface Connection {
  name: string;
  scheme: string;
  host: string | null;
  port: number | null;
  username: string | null;
  root: string;
  options: Record<string, string>;
  encrypted: boolean;
  networked: boolean;
  /** Why every write will be refused, when it will. Null when it will not. */
  rootless: string | null;
  /** What crosses the network unencrypted, as advice. Null when nothing does. */
  clear: string | null;
  /** Where it points, as a person would say it: "nas.local › media". */
  place: string;
  /** The last check, from the window or the command line. */
  last_check: ConnectionCheck | null;
}

export interface ConnectionCheck {
  /** Milliseconds since the Unix epoch. */
  at: number;
  ok: boolean;
  note: string;
}

export type ConnectionForm = Pick<Connection, "name" | "scheme" | "host" | "port" | "username" | "root" | "options">;

/** What still uses a connection. Pairs, syncs and unfinished transfers stop
 *  Delete; history does not, and makes Delete retire it instead. */
export interface ConnectionUses {
  pairs: string[];
  syncs: string[];
  unfinished: string[];
  history: number;
}

export interface Probe {
  entries: number;
  root: string;
  names: string[];
  /** Whether the root will accept a file. Null for a place on this machine.
   *  Listing and writing are different permissions and only the second is what
   *  a drain needs. */
  accepts_files: boolean | null;
  /** An SFTP server nobody has trusted yet: nothing was listed, and the
   *  person is asked whether this fingerprint is their server's. */
  server: ServerKey | null;
}

/** A server's identity, for the person to agree to. */
export interface ServerKey {
  /** As in `SHA256:…`, to compare with what the server itself shows. */
  fingerprint: string;
  /** What is kept with the connection once trusted. */
  key: string;
  /** It is not the key trusted before. */
  changed: boolean;
}

// --- storage -------------------------------------------------------------

export interface ArchiveView {
  name: string;
  archived_at: number;
  size: number;
  /** Null when the archive cannot be read, which the screen shows rather than
   *  hides: an archive that will not open is worth knowing about. */
  links: number | null;
  connections: number | null;
  operations: number | null;
}

/** The choices a link is made of, offered the same way on both surfaces. */
export const CHOICES = {
  source_policy: [
    ["delete", "Delete each once its copy checks out"],
    ["trash", "Send each to the Trash once checked"],
    ["keep", "Keep them (a copy)"],
  ],
  verify: [
    ["hash", "Comparing fingerprints"],
    ["size", "Comparing sizes (fastest, least sure)"],
    ["readback", "Reading it back from the far side"],
  ],
  order: [
    ["largest-first", "Largest first (frees space soonest)"],
    ["smallest-first", "Smallest first (most files soonest)"],
    ["oldest-first", "Oldest first"],
    ["discovered", "As found"],
  ],
  on_conflict: [
    ["quarantine", "Ask me; set it aside if I am away"],
    ["rename", "Keep both"],
    ["skip", "Skip it"],
    ["replace", "Replace it, keeping the old one aside"],
  ],
} as const;

// --- syncs ---------------------------------------------------------------
// Mirrors `crates/tungstate-gui/src/sync.rs`. Counts and reasons cross as data;
// `lib/syncwords.ts` is where they become words.

export interface SyncMember {
  name: string;
  /** `~/CapCut` or `nas:capcut`. */
  at: string;
  remote: boolean;
}

export interface SyncView {
  name: string;
  /** `push`, `pull` or `all`. */
  direction: string;
  exact: boolean;
  /** The anchor's member name, for `push` and `pull`. */
  anchor: string | null;
  on_conflict: string;
  on_remove: string;
  verify: string;
  cooldown_secs: number;
  first_check: string;
  /** `no`, `ask` or `quietly`. */
  launch: string;
  members: SyncMember[];
}

export interface SyncSettingsForm {
  exact: boolean;
  on_conflict: string;
  on_remove: string;
  verify: string;
  cooldown_secs: number;
  first_check: string;
  launch: string;
}

export interface NewSyncForm {
  name: string;
  ends: string[];
  names: string[];
  direction: string;
  anchor: string | null;
  settings: SyncSettingsForm;
}

/** Why a sync could not be made or changed. */
export type SyncRefused =
  | { kind: "delete_needs_exact" }
  | { kind: "replace_has_no_meaning" }
  | { kind: "unknown"; setting: string; value: string }
  | { kind: "all_has_no_anchor" }
  | { kind: "no_such_anchor"; member: string }
  | { kind: "more_names_than_folders" }
  | { kind: "too_few" }
  | { kind: "not_a_folder"; folder: string }
  | { kind: "bad_end"; folder: string; why: string }
  | { kind: "same_name"; member: string }
  | { kind: "overlap"; first: string; second: string }
  | { kind: "taken"; name: string }
  | { kind: "other"; why: string };

/** What a person has said about particular paths before a run. */
export interface SyncAsk {
  resolve?: { path: string; keep: string | null }[];
  forget?: string[];
  only?: boolean;
}

export interface MemberPreview {
  name: string;
  at: string;
  arriving: number;
  arriving_bytes: number;
  leaving: number;
  replacing: number;
  removing: number;
  deleting: number;
  renaming: number;
  parking: number;
  holds: number;
}

export interface SyncLeg {
  from: string;
  to: string;
  files: number;
  parked: number;
  bytes: number;
  through_here: boolean;
}

export interface SyncRemoved {
  member: string;
  path: string;
  /** `deleted`, `extra` or `forgotten`. */
  because: string;
  /** Deleted outright rather than set aside. */
  gone: boolean;
}

export interface SyncConflict {
  path: string;
  versions: { member: string; size: number; modified: number | null }[];
}

export interface SyncLeftAlone {
  path: string;
  /** `too_recent`, `only_anchor_sends`, `only_anchor_receives`, `conflict`,
   *  `case_clash` or `needs_rename`. */
  why: string;
  members: string[];
  existing: string | null;
}

export type SyncRefusal =
  | { kind: "hollow"; member: string; held: number }
  | { kind: "blast"; member: string; taking_off: number; of: number };

export interface SyncPreview {
  sync: string;
  fingerprint: string;
  empty: boolean;
  reversible: boolean;
  members: MemberPreview[];
  legs: SyncLeg[];
  read: { files: number; sampled: number; no_times: number; moves: number; parked: number };
  removed: SyncRemoved[];
  conflicts: SyncConflict[];
  left_alone: SyncLeftAlone[];
  refusals: SyncRefusal[];
}

export interface SyncQueued {
  started: boolean;
  waiting: number;
}

export interface SyncLegEvent {
  sync: string;
  index: number;
  from: string;
  to: string;
}

export interface SyncRan {
  sync: string;
  plan: number | null;
  legs: { from: string; to: string; copied: number; bytes: number; failed: number }[];
  taken_off: number;
  renamed: number;
  missed: { member: string; path: string; why: string }[];
  stopped: boolean;
  reversible: boolean;
}

export interface SyncError {
  sync: string;
  /** `changed`, `refused` or `failed`. */
  kind: string;
  message: string;
}

export interface PastSync {
  plan: number;
  sync: string;
  applied_at: number;
  copied: number;
  taken_off: number;
  renamed: number;
  bytes: number;
  undone: boolean;
  undoable: boolean;
}

export interface SyncUndone {
  plan: number;
  put_back: number;
  taken_off: number;
  parked_left: number;
  revived: { member: string; path: string }[];
}

/** One member of a sync kept in step, and what is happening to it. */
export interface MemberStatus {
  sync: string;
  member: string;
  /** `watching`, `polling` or `paused`. */
  state: string;
  every_secs: number | null;
  why: string | null;
}

export interface FollowingView {
  members: MemberStatus[];
  /** Syncs that stopped to ask, by name. */
  held: string[];
}

/** What restarting into an update would cut short. */
export type Blocker = "transfer" | "sync";
