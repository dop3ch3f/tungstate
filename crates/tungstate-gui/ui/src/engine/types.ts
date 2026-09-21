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

// --- history -------------------------------------------------------------

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
  waiting: number;
}

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
}

export type ConnectionForm = Omit<Connection, "encrypted" | "networked" | "rootless">;

export interface Probe {
  entries: number;
  root: string;
  names: string[];
  /** Whether the root will accept a file. Null for a place on this machine.
   *  Listing and writing are different permissions and only the second is what
   *  a drain needs. */
  accepts_files: boolean | null;
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
    ["delete", "move, deleting each original once verified"],
    ["trash", "move, sending each original to the trash"],
    ["keep", "copy, leaving every original alone"],
  ],
  verify: [
    ["hash", "hash: compare fingerprints"],
    ["size", "size: fastest, least thorough"],
    ["readback", "readback: read it back from the far side"],
  ],
  order: [
    ["largest-first", "largest first: frees space soonest"],
    ["smallest-first", "smallest first: most files soonest"],
    ["oldest-first", "oldest first"],
    ["discovered", "as found"],
  ],
  on_conflict: [
    ["quarantine", "quarantine: set it aside for you"],
    ["rename", "rename: keep both"],
    ["skip", "skip: leave it here"],
    ["replace", "replace: move the existing one aside"],
  ],
} as const;
