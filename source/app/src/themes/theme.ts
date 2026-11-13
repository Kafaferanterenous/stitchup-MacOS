//! Colour schemes (the nine from the Windows build, see schemes.css). Each
//! scopes its tokens to `:root[data-theme="<name>"]` (Light Grey is the bare `:root`
//! default), so switching is just setting the attribute — in any order. The
//! choice is remembered per machine (best effort).

export type ThemeName =
  | "light-grey"
  | "dark-blue"
  | "darker-navy"
  | "blue"
  | "pastel"
  | "hc-dark"
  | "hc-light"
  | "pastel-2"
  | "pastel-3";

export const THEMES: { name: ThemeName; label: string; dark: boolean }[] = [
  { name: "light-grey", label: "Light Grey", dark: false },
  { name: "dark-blue", label: "Dark Blue", dark: true },
  { name: "darker-navy", label: "Darker Navy", dark: true },
  { name: "blue", label: "Blue", dark: false },
  { name: "pastel", label: "Pastel", dark: false },
  { name: "hc-dark", label: "High Contrast (Dark)", dark: true },
  { name: "hc-light", label: "High Contrast (Light)", dark: false },
  { name: "pastel-2", label: "Pastel 2", dark: false },
  { name: "pastel-3", label: "Pastel 3", dark: false },
];

const KEY = "stitchup.theme";

export async function applyTheme(name: ThemeName): Promise<void> {
  document.documentElement.dataset.theme = name;
  try {
    localStorage.setItem(KEY, name);
  } catch {
    /* storage unavailable: theme just isn't remembered */
  }
}

export function currentTheme(): ThemeName {
  return (document.documentElement.dataset.theme as ThemeName) ?? "light-grey";
}

/** ⌘D (as Ctrl+D on Windows): flip between Light Grey and Dark Blue. */
export function toggleDark(): Promise<void> {
  return applyTheme(currentTheme() === "dark-blue" ? "light-grey" : "dark-blue");
}

/** Re-apply the remembered theme at startup. */
export async function restoreTheme(): Promise<void> {
  let saved: string | null = null;
  try {
    saved = localStorage.getItem(KEY);
  } catch {
    saved = null;
  }
  if (saved && THEMES.some((t) => t.name === saved)) await applyTheme(saved as ThemeName);
}
