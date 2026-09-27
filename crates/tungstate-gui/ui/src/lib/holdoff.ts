/** A control that appears where another one was just clicked ignores clicks
 *  for a moment, so the second click of a double click cannot land on it. */
export function holdoff(ms: number, now: () => number = Date.now) {
  let since = Number.NEGATIVE_INFINITY;
  return {
    /** The control has just appeared. */
    start() {
      since = now();
    },
    ready() {
      return now() - since >= ms;
    },
  };
}
