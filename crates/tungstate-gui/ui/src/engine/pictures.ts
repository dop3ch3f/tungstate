// Where the window gets the small pictures a scan made.
//
// They arrive over a scheme of their own rather than inside a command's
// answer: fifty photographs as base64 is most of a megabyte of JSON on every
// redraw, and this is a file on disk being asked for by name.

/** The address of one small picture. */
export function picture(name: string): string {
  // Windows serves custom schemes over http on a made-up host; everywhere else
  // uses the scheme directly. This is Tauri's own arrangement, not ours.
  const encoded = encodeURIComponent(name);
  return navigator.userAgent.includes("Windows")
    ? `http://thumb.localhost/${encoded}`
    : `thumb://localhost/${encoded}`;
}
