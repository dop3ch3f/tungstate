// Which look the window wears.
//
// Kept in the webview's own storage rather than the journal: it is a
// preference about this window, not something the engine remembers, and it
// has to be known before the first paint so the window never flashes the
// wrong colours while a command answers.

export const THEMES = [
  { id: "system", label: "Match the system", note: "Retro, light or dark with your Mac" },
  { id: "retro", label: "Retro", note: "Cream, outlined, hard shadows" },
  { id: "retro-dark", label: "Retro dark", note: "The same shapes after dark" },
  { id: "graphite", label: "Graphite", note: "Dark and smooth" },
  { id: "paper", label: "Paper", note: "Light and quiet" },
] as const;

export type Choice = (typeof THEMES)[number]["id"];
export type Tone = "light" | "dark";
/** What goes on <html>: `data-theme` picks the shapes, `data-tone` the palette. */
export interface Look {
  theme: "retro" | "graphite" | "paper";
  tone: Tone;
}

export const KEY = "tungstate.theme";

export function choiceOf(stored: string | null): Choice {
  return THEMES.some((t) => t.id === stored) ? (stored as Choice) : "system";
}

export function resolve(choice: Choice, systemDark: boolean): Look {
  switch (choice) {
    case "system":
      return { theme: "retro", tone: systemDark ? "dark" : "light" };
    case "retro":
      return { theme: "retro", tone: "light" };
    case "retro-dark":
      return { theme: "retro", tone: "dark" };
    case "graphite":
      return { theme: "graphite", tone: "dark" };
    case "paper":
      return { theme: "paper", tone: "light" };
  }
}
