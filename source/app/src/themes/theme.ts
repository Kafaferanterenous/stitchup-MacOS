//! Theme loader. Default (xp) is statically imported in main.tsx; switching
//! imports the target sheet dynamically (later-in-DOM wins at equal specificity
//! since every theme defines the same tokens on `:root`).

export type ThemeName = "xp" | "dark" | "blue";

export const THEMES: { name: ThemeName; label: string }[] = [
  { name: "xp", label: "Classic (XP)" },
  { name: "dark", label: "Dark" },
  { name: "blue", label: "Blue" },
];

const loaders: Record<ThemeName, () => Promise<unknown>> = {
  xp: () => import("../themes/xp.css"),
  dark: () => import("../themes/dark.css"),
  blue: () => import("../themes/blue.css"),
};

export async function applyTheme(name: ThemeName): Promise<void> {
  await loaders[name]();
  document.documentElement.dataset.theme = name;
}

export function currentTheme(): ThemeName {
  return (document.documentElement.dataset.theme as ThemeName) ?? "xp";
}