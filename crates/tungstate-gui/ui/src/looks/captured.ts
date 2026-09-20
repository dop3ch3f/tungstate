// Real answers from the real engine, so the mockups cannot flatter themselves.
//
// Every number, name, path and sentence below came out of `tungstate plan
// --json` and `tungstate folder learn` run against `messy-downloads/` from
// `scripts/demo-folder.sh`. Nothing here was typed to fit a layout, which is
// the whole point: a mockup fed invented data gets comfortable string lengths
// and tidy two-digit numbers, and then the real screen arrives with a
// 60-character filename and a nine-digit byte count.
//
// Deleted with the rest of `src/looks/` once a direction is chosen.

/** Mirrors `compare::Outcome`. */
export type Outcome = {
  name: string;
  summary: string;
  loads: boolean;
  settles: boolean;
  files: number;
  of: number;
  bytes: number;
  created: number;
  removed: number;
  example: { from: string; to: string } | null;
};

/** Mirrors `govern::LearnedView`. */
export type Learned = {
  levels: string[];
  explains: number;
  of: number;
  loose: number;
  as_is: string;
  improved: string | null;
  suggestions: { headline: string; why: string }[];
};

export const FOLDER = "messy-downloads";
export const FOLDER_PATH = "/Users/you/Downloads";

export const learned: Learned = {
  levels: ["name"],
  explains: 2,
  of: 30,
  loose: 28,
  as_is: "# Written by `tungstate folder learn`…",
  improved: "# The same rules, with every suggestion applied…",
  suggestions: [
    {
      headline: "Give the loose files a home.",
      why: "28 of 30 file(s) sit at the top of the folder, outside the shape.",
    },
    {
      headline: "File the Screenshots files by their names.",
      why: "1 unfiled file(s) are named the way Screenshots names them (`^(Screenshot|Screen Shot|Bildschirmfoto)[ _-]`), which says where they belong without any directory to read.",
    },
  ],
};

export const outcomes: Outcome[] = [
  {
    name: "downloads",
    summary: "Sort by what each file is",
    loads: true,
    settles: true,
    files: 28,
    of: 31,
    bytes: 1121481406,
    created: 6,
    removed: 1,
    example: {
      from: "Screenshot 2026-09-01 at 11.02.44.png",
      to: "Images/Screenshot 2026-09-01 at 11.02.44.png",
    },
  },
  {
    name: "photos",
    summary: "File photos by date",
    loads: true,
    settles: true,
    files: 10,
    of: 31,
    bytes: 973520000,
    created: 6,
    removed: 1,
    example: { from: "Images/already-filed.jpg", to: "2026/07/already-filed.jpg" },
  },
  {
    name: "documents",
    summary: "Group documents by kind, then year",
    loads: true,
    settles: true,
    files: 29,
    of: 31,
    bytes: 1121981406,
    created: 7,
    removed: 2,
    example: { from: "Images/already-filed.jpg", to: "Unsorted/already-filed.jpg" },
  },
  {
    name: "media",
    summary: "Year, app, kind, extension, size",
    loads: true,
    settles: true,
    files: 10,
    of: 31,
    bytes: 973520000,
    created: 12,
    removed: 1,
    example: {
      from: "Images/already-filed.jpg",
      to: "2026/Camera/Photo/jpg/under-100MB/already-filed.jpg",
    },
  },
  {
    name: "by-date",
    summary: "Everything by year, then month",
    loads: true,
    settles: true,
    files: 29,
    of: 31,
    bytes: 1121981406,
    created: 10,
    removed: 2,
    example: { from: "Images/already-filed.jpg", to: "2026/07/already-filed.jpg" },
  },
  {
    name: "by-source",
    summary: "One directory per app it came from",
    loads: true,
    settles: true,
    files: 1,
    of: 31,
    bytes: 310000,
    created: 1,
    removed: 0,
    example: {
      from: "Screenshot 2026-09-01 at 11.02.44.png",
      to: "Screenshots/Screenshot 2026-09-01 at 11.02.44.png",
    },
  },
  {
    name: "by-type",
    summary: "One directory per file extension",
    loads: true,
    settles: true,
    files: 29,
    of: 31,
    bytes: 1121981406,
    created: 10,
    removed: 2,
    example: { from: "Images/already-filed.jpg", to: "JPG/already-filed.jpg" },
  },
];

// The two rows a mockup would never think to draw, and which decide whether a
// layout survives contact. Both are real: `unsettled/` in the demo folder
// produces the first, `broken-rules/` the second.
export const refusals: Outcome[] = [
  {
    name: "(the rules you have)",
    summary: "",
    loads: true,
    settles: false,
    files: 0,
    of: 31,
    bytes: 0,
    created: 0,
    removed: 0,
    example: null,
  },
  {
    name: "(the rules you have)",
    summary: "",
    loads: false,
    settles: false,
    files: 0,
    of: 31,
    bytes: 0,
    created: 0,
    removed: 0,
    example: null,
  },
];

/** Bytes as a person reads them. Copied from the current `api.ts`. */
export function bytes(n: number): string {
  if (n < 1000) return `${n} B`;
  const units = ["kB", "MB", "GB", "TB"];
  let value = n / 1000;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}
