// What the engine pushes at the window, rather than what the window asks for.
//
// Transfers emit; tidying does not. `docs/SEAM.md` lists progress-for-a-tidy
// as a known gap, so a long tidy is one blocking call that returns `TidyDone`
// at the end and the screen must say so rather than draw a bar it cannot fill.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type * as T from "./types";

const on = <P>(name: string) => (f: (payload: P) => void) =>
  listen<P>(name, (e) => f(e.payload));

/** The only folder pass that reports progress. A drive scan takes minutes,
 *  and `docs/SEAM.md` had this listed as a gap. */
export const dupeEvents = {
  progress: on<T.ScanProgress>("dupes://progress"),
};

/** The watcher, while the window is open. */
export const watchEvents = {
  noticed: on<T.Notice>("watch://noticed"),
};

export const transferEvents = {
  queued: on<T.Accepted>("transfer://queued"),
  began: on<T.Began>("transfer://began"),
  planned: on<T.PlannedFile[]>("transfer://planned"),
  advanced: on<T.Advanced>("transfer://advanced"),
  checking: on<T.Advanced>("transfer://checking"),
  atOnce: on<number>("transfer://at-once"),
  started: on<T.Started>("transfer://started"),
  finished: on<T.Finished>("transfer://finished"),
  conflict: on<T.ConflictAsk>("transfer://conflict"),
  identical: on<T.IdenticalAsk>("transfer://identical"),
  done: on<T.Summary>("transfer://done"),
  error: on<string>("transfer://error"),
};

/** A sync's run: the same file events a transfer sends, under `sync://`, and
 *  a `leg` as each pair of members begins. */
export const syncEvents = {
  leg: on<T.SyncLegEvent>("sync://leg"),
  planned: on<T.PlannedFile[]>("sync://planned"),
  started: on<T.Started>("sync://started"),
  advanced: on<T.Advanced>("sync://advanced"),
  checking: on<T.Advanced>("sync://checking"),
  finished: on<T.Finished>("sync://finished"),
  done: on<T.SyncRan>("sync://done"),
  error: on<T.SyncError>("sync://error"),
  following: on<T.FollowingView>("sync://following"),
};

export type { UnlistenFn };
