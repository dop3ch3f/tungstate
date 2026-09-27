// Writes the `latest.json` the installed app reads to learn of a new version.
//
// Run by the release workflow over the collected artefacts of a tagged
// release. Each platform's update file sits beside a `.sig` made with the
// updater key; the file's URL and the signature's text go in, one entry per
// platform the app is installed on.
//
//   node updater-json.mjs <release dir> <tag> <notes file> > latest.json

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

/** Which update file serves which platform, by how it is named. */
const PLATFORMS = [
  ["darwin-aarch64", (f) => f.endsWith("macos-arm64.app.tar.gz")],
  ["darwin-x86_64", (f) => f.endsWith("macos-x86_64.app.tar.gz")],
  ["windows-x86_64", (f) => f.endsWith("-setup.exe")],
  ["linux-x86_64", (f) => f.endsWith(".AppImage")],
];

/** The manifest, from the names in the release and a way to read a `.sig`.
 *  Throws if a platform has no signed update, so a release can never go out
 *  telling half its users there is nothing new. */
export function manifest({ files, signature, tag, notes, date, repo }) {
  const platforms = {};
  for (const [platform, serves] of PLATFORMS) {
    const file = files.find((f) => serves(f) && files.includes(`${f}.sig`));
    if (!file) throw new Error(`no signed update for ${platform}`);
    platforms[platform] = {
      signature: signature(`${file}.sig`).trim(),
      url: `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(file)}`,
    };
  }
  return { version: tag.replace(/^v/, ""), notes, pub_date: date, platforms };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [dir, tag, notesFile] = process.argv.slice(2);
  const out = manifest({
    files: readdirSync(dir),
    signature: (name) => readFileSync(join(dir, name), "utf8"),
    tag,
    notes: readFileSync(notesFile, "utf8").trim(),
    date: new Date().toISOString(),
    repo: process.env.GITHUB_REPOSITORY ?? "dop3ch3f/tungstate",
  });
  process.stdout.write(`${JSON.stringify(out, null, 2)}\n`);
}
