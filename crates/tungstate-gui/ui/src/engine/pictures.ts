// Where the window gets the small pictures a scan made.
//
// They arrive over a scheme of their own rather than inside a command's
// answer: fifty photographs as base64 is most of a megabyte of JSON on every
// redraw, and this is a file on disk being asked for by name.

/** The address of one small picture. */
export function picture(name: string): string {
  return `${base()}/${encodeURIComponent(name)}`;
}

/** A picture of any file, made when first asked for: `edge` is its longest
 *  side in pixels, 256 for a list and up to 1600 for the viewer. */
export function filePicture(path: string, edge = 256): string {
  return `${base()}/f?p=${encodeURIComponent(path)}&s=${edge}`;
}

// Windows and Android serve custom schemes over http on a made-up host;
// macOS, Linux and iOS use the scheme directly. Tauri's arrangement, not ours.
const base = () => (/Windows|Android/.test(navigator.userAgent) ? "http://thumb.localhost" : "thumb://localhost");
