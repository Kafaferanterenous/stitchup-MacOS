//! File-dialog driven actions shared by the ribbon, sidebar and keyboard
//! shortcuts, so every entry point behaves identically.

import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { getState, store } from "./store";

const PDF = [{ name: "PDF", extensions: ["pdf"] }];

function stem(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? "document.pdf";
  return name.replace(/\.pdf$/i, "") || "document";
}

function dirOf(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i > 0 ? path.slice(0, i) : "";
}

export async function pickAndOpen(): Promise<void> {
  const chosen = await openDialog({ multiple: false, filters: PDF });
  if (typeof chosen === "string") await store.open(chosen);
}

/**
 * Save As. The OS save panel already asks before replacing an existing file,
 * so a confirmed target is written with overwrite=true; the backend still
 * refuses to write over the opened source file.
 */
export async function saveAs(): Promise<boolean> {
  const doc = getState().doc;
  if (!doc || doc.passwordRequired) return false;
  const untitled = doc.path.startsWith("stitchup-untitled:");
  const picked = await saveDialog({
    defaultPath: untitled ? "Untitled.pdf" : `${dirOf(doc.path) ? dirOf(doc.path) + "/" : ""}${stem(doc.path)}.pdf`,
    filters: PDF,
  });
  if (typeof picked !== "string") return false;
  // Picking the open file itself is just a Save.
  if (picked === doc.path) return store.save();
  try {
    await store.commit(picked, true);
    // Like any editor: the saved file becomes the document (⌘S goes there).
    await store.adoptSavedPath(picked);
    return true;
  } catch {
    return false; // surfaced via lastError
  }
}

export async function pickAndMerge(): Promise<void> {
  const chosen = await openDialog({ multiple: false, filters: PDF });
  if (typeof chosen === "string") await store.merge(chosen);
}

/** Save `pages` as a new file chosen by the user. */
export async function extractTo(pages: number[]): Promise<boolean> {
  const doc = getState().doc;
  if (!doc || pages.length === 0) return false;
  const label = pages.length === 1 ? `p${pages[0]}` : `p${pages[0]}-${pages[pages.length - 1]}`;
  const picked = await saveDialog({
    defaultPath: `${dirOf(doc.path) ? dirOf(doc.path) + "/" : ""}${stem(doc.path)}-${label}.pdf`,
    filters: PDF,
  });
  if (typeof picked !== "string") return false;
  return (await store.exportPages(pages, picked)) !== null;
}

/** Split every `every` pages into a folder the user picks. */
export async function splitInto(every: number): Promise<boolean> {
  const doc = getState().doc;
  if (!doc) return false;
  const dir = await openDialog({ directory: true, multiple: false, defaultPath: dirOf(doc.path) || undefined });
  if (typeof dir !== "string") return false;
  return (await store.split(every, dir, stem(doc.path))) !== null;
}

/**
 * Parse a page list like "1-3, 5, 8-6" into unique 1-based page numbers
 * (in the order written). Returns null when anything is out of range.
 */
export function parsePages(input: string, max: number): number[] | null {
  const out: number[] = [];
  for (const raw of input.split(/[,\s]+/).filter(Boolean)) {
    const m = raw.match(/^(\d+)(?:-(\d+))?$/);
    if (!m) return null;
    const a = Number(m[1]);
    const b = m[2] ? Number(m[2]) : a;
    if (a < 1 || b < 1 || a > max || b > max) return null;
    const step = a <= b ? 1 : -1;
    for (let n = a; step > 0 ? n <= b : n >= b; n += step) {
      if (!out.includes(n)) out.push(n);
    }
  }
  return out.length ? out : null;
}
