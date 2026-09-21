// What the engine pushes at the window, rather than what the window asks for.
//
// Transfers emit; tidying does not. `docs/SEAM.md` lists progress-for-a-tidy
// as a known gap, so a long tidy is one blocking call that returns `TidyDone`
// at the end and the screen must say so rather than draw a bar it cannot fill.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type * as T from "./types";

const on = <P>(name: string) => (f: (payload: P) => void) =>
  listen<P>(name, (e) => f(e.payload));

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

export type { UnlistenFn };
