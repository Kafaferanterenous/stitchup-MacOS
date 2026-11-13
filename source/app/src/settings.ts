//! User settings (File ▸ Settings…, ⌘,). Stored per machine in localStorage —
//! a convenience: if storage is unavailable the defaults apply.

export interface Settings {
  /** Crash-recovery snapshot interval in seconds (0 = off, the Windows default). */
  recoverySeconds: number;
  /** Zoom a newly opened document starts in. */
  openZoom: "fit-width" | "fit-page" | "actual";
  /** Show the page sidebar when a document opens. */
  sidebarOnOpen: boolean;
}

const KEY = "stitchup.settings";
export const DEFAULTS: Settings = { recoverySeconds: 0, openZoom: "fit-width", sidebarOnOpen: true };

export function loadSettings(): Settings {
  try {
    return { ...DEFAULTS, ...(JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<Settings>) };
  } catch {
    return { ...DEFAULTS };
  }
}

export function saveSettings(s: Settings) {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    /* not remembered */
  }
  window.dispatchEvent(new CustomEvent("stitchup-settings", { detail: s }));
}
