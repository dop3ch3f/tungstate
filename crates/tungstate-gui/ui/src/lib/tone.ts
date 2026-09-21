// Engine strings never reach a `class` attribute directly.
//
// `TransfersView.vue:218` does `:class="row.state"` and `ActivityView.vue`
// does `:class="op.status"`. Both take a string the Rust side chose and put it
// straight into the DOM. Add an outcome on the engine side and the row renders
// with no rule at all: unstyled, not broken, and nothing fails. It is the same
// family as the four selectors slice 7b found matching nothing, arriving from
// the other direction.
//
// So: a total map from every string the engine can send to a tone this window
// styles, with an explicit fallback. A new engine outcome gets a known look
// instead of none.

export type Tone = "plain" | "live" | "ok" | "hold" | "bad";

const OUTCOME: Record<string, Tone> = {
  transferred: "ok",
  verified: "ok",
  "already-present": "plain",
  already_present: "plain",
  skipped: "plain",
  quarantined: "hold",
  failed: "bad",
  cancelled: "hold",
};

const OP_STATUS: Record<string, Tone> = {
  ok: "ok",
  interrupted: "hold",
  failed: "bad",
  skipped: "plain",
};

/** The look for a finished file's outcome. Unknown strings read as plain
 *  rather than as nothing. */
export const toneOfOutcome = (outcome: string): Tone => OUTCOME[outcome] ?? "plain";

/** The look for a journal op's status. */
export const toneOfStatus = (status: string): Tone => OP_STATUS[status] ?? "plain";
