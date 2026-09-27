/** Only the newest of overlapping requests may land. Take a ticket before the
 *  call; after it, `stale(ticket)` says a newer one was taken meanwhile. */
export function latest() {
  let newest = 0;
  return {
    take: () => ++newest,
    stale: (ticket: number) => ticket !== newest,
  };
}
