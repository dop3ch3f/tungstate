// The section glyphs, each a few strokes on a 16pt grid.
//
// Drawn rather than an icon font: three shapes do not justify a dependency,
// and a path takes the colour of whatever it sits in.

export type Section = "home" | "folder" | "drain" | "sync" | "dupes" | "history" | "connections" | "settings";

export const ICON: Record<Section, string> = {
  home: "M2.5 7.5L8 3l5.5 4.5M4 6.5V13h8V6.5",
  folder: "M1.5 4.5v8h13v-7H7.5L6 4H1.5zM4 9h8",
  drain: "M3 2.5h10v4H3zM3 9.5h10v4H3zM8 6.5v3M6.5 8L8 9.5 9.5 8",
  history: "M8 2a6 6 0 1 0 0 12A6 6 0 0 0 8 2zM8 4.5V8l2.5 1.5",
  settings: "M2.5 4.5h11M2.5 8h11M2.5 11.5h11M5.5 3v3M10.5 6.5v3M7 10v3",
  dupes: "M2.5 2.5h8v8h-8zM5.5 5.5h8v8h-8z",
  // Two arrows chasing each other round: things going both ways.
  sync: "M3 7a5 5 0 0 1 9-2.5M13 9a5 5 0 0 1-9 2.5M12.5 2v3h-3M3.5 14v-3h3",
  // A plug: a place you connect to.
  connections: "M6 2v3.5M10 2v3.5M4 5.5h8V8a4 4 0 0 1-8 0zM8 12v2.5",
};
