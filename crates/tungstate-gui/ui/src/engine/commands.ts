// Every call the window can make into the engine, in one place.
//
// Grouped by the question being asked, the way `docs/SEAM.md` groups them.
// Nothing here formats, words or decides anything: a command sends arguments
// and returns data. Every fallible one rejects with a bare sentence, because
// that is what `Result<T, String>` becomes on this side.

import { invoke } from "@tauri-apps/api/core";
import type * as T from "./types";

export const folders = {
  governed: () => invoke<T.FolderView[]>("governed"),
  govern: (root: string) => invoke<void>("govern_folder", { root }),
  forget: (root: string) => invoke<void>("forget_folder", { root }),
  layouts: () => invoke<T.LayoutView[]>("layouts"),
  /** Writes a first draft. Refuses if the folder already has rules; the window
   *  never edits a policy, because the file lives in the folder and is meant
   *  to be committed. */
  giveRules: (root: string, layout: string) => invoke<void>("give_rules", { root, layout }),
  rulesText: (root: string) => invoke<string>("rules_text", { root }),
  /** Both of these take a bare path and need no registration, which is what
   *  lets somebody point at a folder and be shown their own files before they
   *  have learned a single word of this app's vocabulary. */
  learn: (root: string) => invoke<T.Learned>("learn_folder", { root }),
  compare: (root: string) => invoke<T.Outcome[]>("compare_folder", { root }),
  preview: (root: string) => invoke<T.PreviewView>("folder_preview", { root }),
  tidy: (root: string) => invoke<T.TidyDone>("tidy_folder", { root }),
  /** Takes any plan id, though nothing in the seam enumerates them; see
   *  `docs/SEAM.md` §2. In practice the window offers `PreviewView.undoable`. */
  putBack: (root: string, plan: number) => invoke<T.PutBackDone>("put_back", { root, plan }),
  pick: () => invoke<string | null>("pick_folder"),
};

export const dupes = {
  /** Walks the whole of `target`, which may be a folder or `connection:folder`.
   *  Emits `dupes://progress` as it goes; a long one is expected. */
  find: (target: string) => invoke<T.Found>("find_duplicates", { target }),
  stop: () => invoke<void>("stop_finding_duplicates"),
  /** Confirms every group byte for byte before anything moves. */
  clear: (target: string, only: string[], choices: T.DupeChoice[], extras: string) =>
    invoke<T.Cleared>("clear_duplicates", { target, only, choices, extras }),
  /** What the person said should happen to extra copies, if they have said. */
  action: () => invoke<string | null>("duplicate_action"),
  rememberAction: (action: string) => invoke<void>("set_duplicate_action", { action }),
};

export const history = {
  recent: () => invoke<T.Op[]>("recent"),
  ofPath: (path: string) => invoke<T.Op[]>("history", { path }),
  /** Takes a path spelled any way a person might type it, or a BLAKE3 digest. */
  whereis: (target: string) => invoke<T.Op[]>("whereis", { target }),
  quarantined: (link: string) => invoke<string[]>("quarantined", { link }),
};

export const transfers = {
  browse: (path: string) => invoke<T.Listing>("browse", { path }),
  places: () => invoke<T.Place[]>("places"),
  lastPanes: () => invoke<{ left: string | null; right: string | null }>("last_panes"),
  rememberPanes: (panes: { left: string | null; right: string | null }) =>
    invoke<void>("remember_panes", { panes }),
  preview: (request: T.TransferRequest) => invoke<T.Preview>("preview_transfer", { request }),
  /** Returns the link name it ran under. */
  start: (request: T.TransferRequest) => invoke<string>("start_transfer", { request }),
  cancel: () => invoke<void>("cancel_run"),
  stopNow: () => invoke<void>("stop_now"),
  setAtOnce: (files: number) => invoke<void>("set_at_once", { files }),
  resolveConflict: (action: string, applyToAll: boolean) =>
    invoke<void>("resolve_conflict", { action, applyToAll }),
  resolveIdentical: (remove: boolean, applyToAll: boolean) =>
    invoke<void>("resolve_identical", { remove, applyToAll }),
  interrupted: () => invoke<T.InterruptedRun[]>("interrupted"),
  resume: (link: string) => invoke<T.Accepted>("resume_interrupted", { link }),
  /** Returns the bytes it freed. */
  discard: (link: string) => invoke<number>("discard_interrupted", { link }),
};

export const links = {
  list: () => invoke<T.Link[]>("list_links"),
  create: (form: T.NewLink) => invoke<void>("create_link", { form }),
  run: (name: string) => invoke<T.Accepted>("run_link", { name }),
  preview: (name: string) => invoke<T.Preview>("preview_link", { name }),
  /** True when the link was retired rather than deleted: its history survives. */
  remove: (name: string) => invoke<boolean>("remove_link", { name }),
};

export const connections = {
  list: () => invoke<T.Connection[]>("list_connections"),
  add: (form: T.ConnectionForm, secret: string | null) =>
    invoke<void>("add_connection", { form, secret }),
  update: (name: string, form: T.ConnectionForm) =>
    invoke<void>("update_connection", { name, form }),
  setPassword: (name: string, secret: string) =>
    invoke<void>("set_connection_password", { name, secret }),
  test: (name: string) => invoke<T.Probe>("test_connection", { name }),
  remove: (name: string) => invoke<void>("remove_connection", { name }),
};

export const storage = {
  archives: () => invoke<T.ArchiveView[]>("archives"),
  /** Returns the archive the previous state was put under. */
  reset: () => invoke<string>("reset_storage"),
  restore: (name: string) => invoke<string>("restore_archive", { name }),
  forget: (name: string) => invoke<void>("forget_archive", { name }),
  export: (path: string) => invoke<void>("export_storage", { path }),
  /** Returns the number of rows read. */
  import: (path: string) => invoke<number>("import_storage", { path }),
  pickSaveFile: (suggested: string) => invoke<string | null>("pick_save_file", { suggested }),
  pickOpenFile: () => invoke<string | null>("pick_open_file"),
};
