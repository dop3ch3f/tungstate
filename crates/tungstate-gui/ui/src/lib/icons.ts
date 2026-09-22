// The section glyphs, each a few strokes on a 16pt grid.
//
// Drawn rather than an icon font: three shapes do not justify a dependency,
// and a path takes the colour of whatever it sits in.

export type Section = "home" | "folder" | "drain" | "history";

export const ICON: Record<Section, string> = {
  home: "M2.5 7.5L8 3l5.5 4.5M4 6.5V13h8V6.5",
  folder: "M1.5 4.5v8h13v-7H7.5L6 4H1.5zM4 9h8",
  drain: "M3 2.5h10v4H3zM3 9.5h10v4H3zM8 6.5v3M6.5 8L8 9.5 9.5 8",
  history: "M8 2a6 6 0 1 0 0 12A6 6 0 0 0 8 2zM8 4.5V8l2.5 1.5",
};
