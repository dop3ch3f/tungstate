// A sync in plain words.
//
// The engine keeps its vocabulary (push, pull, all, exact, quarantine) and the
// command line uses it; the window says what those mean, in the words chosen
// for slice 9d. Every sentence about a sync on screen comes from here, so the
// same thing is never said two ways.

import type {
  SyncLeftAlone, SyncRefusal, SyncRefused, SyncRemoved, SyncView,
} from "../engine/types";

/** Which way things go, short: for a row or a chip. */
export function way(sync: Pick<SyncView, "direction" | "anchor">): string {
  const anchor = sync.anchor ?? "the first folder";
  switch (sync.direction) {
    case "push":
      return `Send from ${anchor}`;
    case "pull":
      return `Bring into ${anchor}`;
    default:
      return "Both ways";
  }
}

/** Which way things go, as a sentence. */
export function wayInFull(sync: Pick<SyncView, "direction" | "anchor">): string {
  const anchor = sync.anchor ?? "the first folder";
  switch (sync.direction) {
    case "push":
      return `Send from ${anchor} to the others: everything on ${anchor} is copied to them.`;
    case "pull":
      return `Bring into ${anchor}: everything on the others is copied to ${anchor}.`;
    default:
      return "Both ways: every folder gets everything.";
  }
}

/** Whether deletions travel, short. */
export const exactly = (sync: Pick<SyncView, "exact">): string =>
  sync.exact ? "Deletions too" : "Only add";

/** Whether deletions travel, as a sentence. */
export function exactlyInFull(sync: Pick<SyncView, "exact" | "on_remove">): string {
  if (!sync.exact) return "Only add: nothing is ever removed, and a deleted file comes back.";
  return sync.on_remove === "delete"
    ? "Deletions too: delete on one, and it is deleted on the others. That cannot be put back."
    : "Deletions too: delete on one, and it is set aside on the others, where it can be put back.";
}

export const WAYS = [
  { id: "push", label: "Send", line: "One folder sends to the others. Nothing comes back to it." },
  { id: "pull", label: "Bring in", line: "The others send to one folder. Nothing goes out of it." },
  { id: "all", label: "Both ways", line: "Every folder gets everything." },
] as const;

export const EXACTLY = [
  { id: false, label: "Only add", line: "Nothing is ever removed. A file deleted on one comes back." },
  { id: true, label: "Deletions too", line: "Delete on one, and it goes from the others as well." },
] as const;

export const REMOVING = [
  { id: "set-aside", label: "Set aside", line: "Kept in the folder's set-aside area, where it can be put back." },
  { id: "delete", label: "Delete outright", line: "Gone for good, for a folder that exists to free space. Cannot be put back." },
] as const;

export const CONFLICTS = [
  { id: "quarantine", label: "Keep each, ask me", line: "Every folder keeps its own version, the others are set aside beside it, and you choose later." },
  { id: "rename", label: "Keep both", line: "Every folder gets every version, each named after the folder it came from." },
  { id: "skip", label: "Leave it until I choose", line: "Nothing moves for that file until you settle it." },
] as const;

export const LAUNCH = [
  { id: "no", label: "Only when I press Run", line: "Nothing runs by itself." },
  { id: "ask", label: "When the app opens, show me first", line: "What would move is shown, and nothing moves until you say so." },
  { id: "quietly", label: "When the app opens, just run it", line: "Unless it would remove anything or needs a yes; then it asks." },
  { id: "continuous", label: "Keep in step while the app is open", line: "Runs when the app opens and whenever something changes. Asks first if it would remove anything." },
] as const;

export const FIRST_CHECK = [
  { id: "full", label: "Read both in full", line: "Exact. A first run over a big archive on a network takes as long as reading it." },
  { id: "sampled", label: "Read samples", line: "Start, middle and end. Minutes rather than hours; a difference only between the samples is missed." },
  { id: "size", label: "Trust the size", line: "Reads nothing. Fastest; an edit that kept the size is missed." },
] as const;

/** When a sync runs, as a sentence about it rather than a choice. */
export function whenItRuns(sync: Pick<SyncView, "launch">): string {
  switch (sync.launch) {
    case "ask":
      return "Runs when the app opens, after showing you what would move.";
    case "quietly":
      return "Runs by itself when the app opens, unless it would remove anything.";
    case "continuous":
      return "Kept in step while the app is open: runs whenever something changes, and asks first if it would remove anything.";
    default:
      return "Runs only when you press Run.";
  }
}

/** What is happening to a member of a sync kept in step, in words. */
const every = (secs: number | null) => {
  const s = secs ?? 0;
  return s < 60 ? `${s} seconds` : s === 60 ? "minute" : `${Math.round(s / 60)} minutes`;
};

export function following(status: { state: string; every_secs: number | null; why: string | null }): string {
  switch (status.state) {
    case "watching":
      return "watching for changes";
    case "polling":
      return `checked every ${every(status.every_secs)}`;
    // The engine's reason ("no route to host") is for the tooltip; the words
    // say what is happening about it.
    case "paused":
      return status.every_secs ? `out of reach, trying again every ${every(status.every_secs)}` : "out of reach, trying again on its own";
    default:
      return status.state;
  }
}

/** Why a path was left alone, in words. */
export function why(left: SyncLeftAlone): string {
  const [first, ...rest] = left.members;
  switch (left.why) {
    case "too_recent":
      return `still being written on ${first}`;
    case "only_anchor_sends":
      return `${first} has it, but only the sending folder sends`;
    case "only_anchor_receives":
      return `${first} has it, but it only receives`;
    case "conflict":
      return `changed differently on ${[first, ...rest].join(" and ")}`;
    case "case_clash":
      return `${first} already has ${left.existing ?? "a name"} that differs only in capitals`;
    case "needs_rename":
      return `${first} cannot rename, so its copy cannot be set aside`;
    default:
      return left.why;
  }
}

/** Why a file would be removed, in words. */
export function removedBecause(removed: SyncRemoved, members: string[] = []): string {
  // With two folders, "another folder" is always the same one: name it.
  const others = members.filter((m) => m !== removed.member);
  const elsewhere = others.length === 1 ? others[0] : "another folder";
  const how = removed.gone ? "deleted for good" : "kept in its set-aside area";
  switch (removed.because) {
    case "deleted":
      return `From ${removed.member}, because they were deleted on ${elsewhere}. ${cap(how)}.`;
    case "extra":
      return `From ${removed.member}, because the folder it follows does not have them. ${cap(how)}.`;
    case "forgotten":
      return `From ${removed.member}, because you asked to forget them. ${cap(how)}.`;
    default:
      return how;
  }
}

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** Why a file was not done in a run, in words. The runner's reasons for the
 *  common cases are its own sentences; these are the window's. */
export function missed(m: { member: string; why: string }): string {
  if (m.why.startsWith("it has been written to")) return `Changed on ${m.member} during the run. It will be tried next time.`;
  if (m.why.startsWith("it is no longer there")) return `Gone from ${m.member} during the run. It will be looked at again next time.`;
  if (m.why.includes("is already taken")) return `Its new name on ${m.member} was taken during the run. It will be tried next time.`;
  return `On ${m.member}: ${m.why}. It will be tried next time.`;
}

/** Why a run needs a yes, in words. */
export function refusal(refused: SyncRefusal): string {
  switch (refused.kind) {
    case "hollow":
      return `${refused.member} looks empty, but held ${refused.held} ${refused.held === 1 ? "file" : "files"} after the last run. An unplugged drive looks the same as everything deleted.`;
    case "blast":
      return `This would remove ${refused.taking_off} of the ${refused.of} ${refused.of === 1 ? "file" : "files"} on ${refused.member}.`;
  }
}

/** Why a sync could not be made or changed, in words. */
export function refused(r: SyncRefused): string {
  switch (r.kind) {
    case "delete_needs_exact":
      return "Deleting outright only makes sense with Deletions too.";
    case "replace_has_no_meaning":
      return "Replacing cannot say whose version wins when several folders changed a file.";
    case "unknown":
      return `The setting ${r.setting} cannot be ${r.value}.`;
    case "all_has_no_anchor":
      return "Both ways has no folder that sends for the others.";
    case "no_such_anchor":
      return `${r.member} is not one of the folders.`;
    case "more_names_than_folders":
      return "There are more names than folders.";
    case "too_few":
      return "A sync needs two folders or more.";
    case "not_a_folder":
      return `${r.folder} is not a folder on this Mac.`;
    case "bad_end":
      return r.why;
    case "same_name":
      return `Two folders would both be called ${r.member}. Give one another name.`;
    case "overlap":
      return `${r.first} and ${r.second} overlap: one folder cannot sit inside another.`;
    case "taken":
      return `There is already a sync called ${r.name}.`;
    case "other":
      return r.why;
  }
}

/** A rejected command's reason: a `SyncRefused` when the engine refused as
 *  data, otherwise the sentence it sent. */
export function reason(e: unknown): string {
  if (e && typeof e === "object" && "kind" in e) return refused(e as SyncRefused);
  return String(e);
}
