// What a connection's failure means, in one sentence a person can act on.
// The engine's text is written for the command line and passes on what the
// protocol library said, context and all; the phone shows this instead and
// keeps the original behind Details. Pure, so the wording can be tested.

export interface Plain {
  says: string;
  /** The original text, when `says` is not it. */
  detail: string | null;
}

/** Longer than this, or carrying a library's context dump, and the original
 *  text is not something to put in front of anyone. */
const READABLE = 140;

// First match wins, so the specific ones come before the general.
const RULES: [RegExp, (where: string) => string][] = [
  [/refuses files|accepts? no files|permission denied|\b55[03]\b.*(write|stor|create|permission)/i, () => "That folder doesn't take new files with this login. Choose another."],
  [/\b530\b|login incorrect|not logged in|refused the credentials|wrong password|authenticat|logon failure/i, () => "The username or password wasn't accepted."],
  [/timed? ?out|did not answer|deadline/i, (w) => `${w} didn't answer. Check this phone is on the same Wi-Fi.`],
  [/connection refused|refused the connection|ECONNREFUSED/i, (w) => `${w} refused the connection. Check the address and port.`],
  [/no route|no longer reachable|unreachable|failed to lookup|name or service not known|dns|resolve/i, (w) => `Couldn't find ${w}. Check the address and that this phone is on the same Wi-Fi.`],
  [/\b550\b|no such file|not found|does not exist/i, () => "That folder isn't there."],
];

/** One sentence for a failure at `where` ("Area51", "the server"). */
export function plainError(raw: unknown, where = "The server"): Plain {
  const text = String(raw instanceof Error ? raw.message : raw).trim();
  for (const [pattern, say] of RULES) if (pattern.test(text)) return { says: say(where), detail: text };
  if (text.length <= READABLE && !/context:|\{/.test(text)) return { says: text, detail: null };
  return { says: `Something went wrong talking to ${where}.`, detail: text };
}
