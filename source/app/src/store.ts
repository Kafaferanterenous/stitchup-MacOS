//! Tiny app store + CommandBus around the adapters.
//!
//! Undo/redo (A-c8): the Rust core journals an exact before/after snapshot
//! (`stitchup_core::oplog`) around every structural mutation, so undo/redo here
//! are thin `undo`/`redo` IPC calls — a mixed sequence (reorder + rotate +
//! delete + insert + edit_text + set_metadata + merge) reverts EXACTLY at every
//! step. The frontend `history`/`redo` Operation stacks are mirrors kept only
//! so the toolbar can enable/disable the buttons; the Rust `OpLog` is the
//! source of truth.

import { useSyncExternalStore } from "react";
import type { Document, Operation, TextEditParams } from "./types";
import {
  commitAdapter,
  documentFromOpen,
  metadataAdapter,
  openDocumentAdapter,
  pageTreeAdapter,
} from "./adapters/structuralAdapter";
import { cmdOpen, cmdRedo, cmdRenderBytes, cmdUndo, cmdUnlock } from "./ipc";

export interface AppState {
  doc: Document | null;
  currentPage: number;
  zoom: number;
  zoomMode: "fit-page" | "fit-width" | "actual" | "custom";
  viewMode: "single" | "spread" | "continuous";
  busy: boolean;
  lastError: Error | null;
  /** Transient, non-blocking notice (e.g. "Copied 12 characters"). */
  notice: string | null;
  lastCommit: { target: string; sha: string; size: number } | null;
  statusMessage: string;
  metadataOpen: boolean;
  /** Text-edit mode: viewer overlays clickable text runs. */
  editMode: boolean;
  /** Current text-layer selection (logical characters), for Copy (A-c10). */
  selectionText: string;
  /** Left pane (thumbnails / bookmarks). */
  paneOpen: boolean;
  paneTab: "pages" | "bookmarks";
  /** Password prompt for a protected document (A-c6). */
  unlockOpen: boolean;
}

let state: AppState = {
  doc: null,
  currentPage: 1,
  zoom: 1,
  zoomMode: "fit-page",
  viewMode: "continuous",
  busy: false,
  lastError: null,
  notice: null,
  lastCommit: null,
  statusMessage: "No document open",
  metadataOpen: false,
  editMode: false,
  selectionText: "",
  paneOpen: true,
  paneTab: "pages",
  unlockOpen: false,
};

const listeners = new Set<() => void>();

let noticeTimer: ReturnType<typeof setTimeout> | null = null;

function set(partial: Partial<AppState>) {
  state = { ...state, ...partial };
  listeners.forEach((l) => l());
}

/** Show a transient notice that clears itself after a moment. */
function notify(message: string) {
  set({ notice: message });
  if (noticeTimer) clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => set({ notice: null }), 2500);
}

export function getState(): AppState {
  return state;
}

export function subscribe(l: () => void): () => void {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function useAppState(): AppState {
  return useSyncExternalStore(subscribe, getState);
}

async function refreshPages(doc: Document): Promise<Document> {
  return documentFromOpen(await cmdOpen(doc.path), doc);
}

/** Re-serialize the mutated document so the renderer rebinds to it. */
async function withFreshBytes(doc: Document): Promise<Document> {
  if (doc.passwordRequired || doc.encrypted) return doc;
  try {
    const rb = await cmdRenderBytes();
    return { ...doc, bytesB64: rb.bytes_b64 };
  } catch {
    // Best-effort: keep the last known bytes if serialization fails.
    return doc;
  }
}

/** Keep the page index inside the (possibly shorter) document. */
function clampPage(doc: Document, wanted: number): number {
  return Math.min(Math.max(1, wanted), Math.max(1, doc.pages.length));
}

/** execCommand('copy') fallback for webviews without the async Clipboard API. */
function legacyCopy(text: string): boolean {
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.setAttribute("readonly", "");
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  } finally {
    document.body.removeChild(ta);
  }
  return ok;
}

export const store = {
  async open(path: string) {
    set({ busy: true, lastError: null });
    try {
      const doc = await openDocumentAdapter.open(path);
      set({
        doc,
        currentPage: 1,
        editMode: false,
        selectionText: "",
        unlockOpen: doc.passwordRequired,
        statusMessage: doc.passwordRequired
          ? "This document is password-protected"
          : `Opened ${doc.pages.length} pages`,
      });
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Open failed" });
    } finally {
      set({ busy: false });
    }
  },

  /** Supply the document password (A-c6); clears the password gate on success. */
  async unlock(password: string) {
    set({ busy: true, lastError: null });
    try {
      const doc = documentFromOpen(await cmdUnlock(password));
      set({
        doc,
        currentPage: 1,
        unlockOpen: false,
        selectionText: "",
        statusMessage: `Unlocked ${doc.pages.length} pages`,
      });
      return true;
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Unlock failed" });
      return false;
    } finally {
      set({ busy: false });
    }
  },

  setUnlockOpen(open: boolean) {
    set({ unlockOpen: open });
  },

  /** Run a PageTreeAdapter mutation and record its Operation. */
  async command(op: Operation, mutation: (doc: Document) => Promise<unknown>): Promise<boolean> {
    set({ busy: true, lastError: null });
    try {
      const doc = state.doc;
      if (!doc) throw new Error("no document open");
      await mutation(doc);
      let next = await refreshPages(doc);
      next = await withFreshBytes(next);
      set({
        doc: { ...next, history: [...next.history, op], redo: [] },
        currentPage: clampPage(next, state.currentPage),
        statusMessage: `Applied ${op.kind}`,
      });
      return true;
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Command failed" });
      return false;
    } finally {
      set({ busy: false });
    }
  },

  /** Structural action helpers — each builds an Operation and runs it. */
  async reorder(newOrder: number[]) {
    const doc = state.doc;
    if (!doc) return false;
    const before = doc.pages.map((p) => p.page_no);
    return this.command(
      { kind: "reorder", before, data: { new_order: newOrder } },
      () => pageTreeAdapter.reorder(doc, newOrder),
    );
  },

  async rotate(pages: number[], cw: boolean) {
    const doc = state.doc;
    if (!doc) return false;
    const before = doc.pages.map((p) => p.page_no);
    return this.command(
      { kind: "rotate", before, data: { pages, cw } },
      () => pageTreeAdapter.rotate(doc, pages, cw),
    );
  },

  async deletePages(pages: number[]) {
    const doc = state.doc;
    if (!doc) return false;
    const before = doc.pages.map((p) => p.page_no);
    return this.command(
      { kind: "delete", before, data: { pages } },
      () => pageTreeAdapter.delete(doc, pages),
    );
  },

  async insertBlank(afterPage: number, widthPt: number, heightPt: number) {
    const doc = state.doc;
    if (!doc) return false;
    const before = doc.pages.map((p) => p.page_no);
    return this.command(
      { kind: "insert_blank", before, data: { after: afterPage, width_pt: widthPt, height_pt: heightPt } },
      () => pageTreeAdapter.insertBlank(doc, afterPage, widthPt, heightPt),
    );
  },

  async editText(params: TextEditParams) {
    const doc = state.doc;
    if (!doc || doc.passwordRequired || doc.encrypted) return false;
    const before = doc.pages.map((p) => p.page_no);
    return this.command(
      { kind: "edit_text", before, data: params },
      () => pageTreeAdapter.editText(doc, params),
    );
  },

  /** Append another PDF's pages to this document (A-c15). */
  async merge(path: string) {
    const doc = state.doc;
    if (!doc || doc.passwordRequired) return false;
    const before = doc.pages.map((p) => p.page_no);
    return this.command(
      { kind: "merge", before, data: { path } },
      () => pageTreeAdapter.merge(doc, path),
    );
  },

  async extract(pages: number[]) {
    const out = await pageTreeAdapter.extract(
      state.doc!,
      pages,
    );
    return out;
  },

  /** Undo the last structural mutation — exact restore (A-c8). */
  async undo(): Promise<boolean> {
    const doc = state.doc;
    if (!doc || doc.passwordRequired || doc.history.length === 0) return false;
    const op = doc.history.at(-1)!;
    set({ busy: true, lastError: null });
    try {
      await cmdUndo();
      let next = await refreshPages(doc);
      next = await withFreshBytes(next);
      set({
        doc: { ...next, history: doc.history.slice(0, -1), redo: [...doc.redo, op] },
        currentPage: clampPage(next, state.currentPage),
        statusMessage: `Undid ${op.kind}`,
      });
      return true;
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Undo failed" });
      return false;
    } finally {
      set({ busy: false });
    }
  },

  /** Redo the most recently undone mutation — exact replay (A-c8). */
  async redo(): Promise<boolean> {
    const doc = state.doc;
    if (!doc || doc.passwordRequired || doc.redo.length === 0) return false;
    const op = doc.redo.at(-1)!;
    set({ busy: true, lastError: null });
    try {
      await cmdRedo();
      let next = await refreshPages(doc);
      next = await withFreshBytes(next);
      set({
        doc: { ...next, history: [...doc.history, op], redo: doc.redo.slice(0, -1) },
        currentPage: clampPage(next, state.currentPage),
        statusMessage: `Redid ${op.kind}`,
      });
      return true;
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Redo failed" });
      return false;
    } finally {
      set({ busy: false });
    }
  },

  async commit(target: string, overwrite: boolean) {
    set({ busy: true, lastError: null });
    try {
      const doc = state.doc;
      if (!doc) throw new Error("no document open");
      const res = await commitAdapter.export(doc, target, overwrite);
      set({
        lastCommit: { target: res.published_path, sha: res.new_sha256, size: res.size_bytes },
        statusMessage: `Saved to ${res.published_path}`,
      });
      return res;
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Save failed" });
      throw e;
    } finally {
      set({ busy: false });
    }
  },

  async updateMetadata(updates: Parameters<typeof metadataAdapter.set>[1]) {
    const doc = state.doc;
    if (!doc) return;
    set({ busy: true, lastError: null });
    try {
      const prev = doc.metadata;
      await metadataAdapter.set(doc, updates);
      let next = await refreshPages(doc);
      next = await withFreshBytes(next);
      set({
        doc: {
          ...next,
          metadata: { ...next.metadata, ...updates },
          history: [
            ...next.history,
            { kind: "set_metadata", before: next.pages.map((p) => p.page_no), beforeMeta: prev, data: updates },
          ],
          redo: [],
        },
        statusMessage: "Metadata updated",
      });
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Metadata update failed" });
    } finally {
      set({ busy: false });
    }
  },

  setZoom(z: number, mode: AppState["zoomMode"]) {
    set({ zoom: z, zoomMode: mode });
  },

  setViewMode(mode: AppState["viewMode"]) {
    set({ viewMode: mode });
  },

  setPaneOpen(open: boolean) {
    set({ paneOpen: open });
  },

  setPaneTab(tab: AppState["paneTab"]) {
    // Selecting a tab also re-opens the pane if it was hidden.
    set({ paneTab: tab, paneOpen: true });
  },

  async addBookmark() {
    const doc = state.doc;
    if (!doc) return;
    const n = state.currentPage;
    if (doc.bookmarks.includes(n)) return;
    const next: Document = {
      ...doc,
      bookmarks: [...doc.bookmarks, n].sort((a, b) => a - b),
    };
    set({ doc: next, statusMessage: `Bookmarked page ${n}` });
  },

  async removeBookmark(n: number) {
    const doc = state.doc;
    if (!doc) return;
    const next: Document = {
      ...doc,
      bookmarks: doc.bookmarks.filter((b) => b !== n),
    };
    set({ doc: next, statusMessage: "Bookmark removed" });
  },

  async clearBookmarks() {
    const doc = state.doc;
    if (!doc) return;
    const next: Document = { ...doc, bookmarks: [] };
    set({ doc: next, statusMessage: "All bookmarks removed" });
  },

  goToPage(n: number) {
    const doc = state.doc;
    if (!doc || n < 1 || n > doc.pages.length) return;
    set({ currentPage: n });
  },

  setMetadataOpen(open: boolean) {
    set({ metadataOpen: open });
  },

  setEditMode(on: boolean) {
    set({ editMode: on });
  },

  /** Track the current text-layer selection (called from selectionchange). */
  setSelection(text: string) {
    if (text === state.selectionText) return;
    set({ selectionText: text });
  },

  /**
   * Copy the current selection to the OS clipboard, Unicode-preserving (A-c10).
   * Uses the async Clipboard API, falling back to a hidden textarea + execCommand
   * for webviews that gate `navigator.clipboard`.
   */
  async copySelection(): Promise<boolean> {
    const text = state.selectionText || document.getSelection()?.toString() || "";
    if (!text) return false;
    let ok = false;
    try {
      await navigator.clipboard.writeText(text);
      ok = true;
    } catch {
      ok = legacyCopy(text);
    }
    set({
      statusMessage: ok
        ? `Copied ${text.length} character${text.length === 1 ? "" : "s"}`
        : "Copy failed",
    });
    notify(ok ? `Copied ${text.length} character${text.length === 1 ? "" : "s"}` : "Copy failed");
    return ok;
  },

  clearError() {
    set({ lastError: null });
  },
};

/** Keyboard shortcut map (accessibility, A-c list). */
export const SHORTCUTS = {
  open: "meta+o",
  saveAs: "meta+shift+s",
  undo: "meta+z",
  redo: "meta+shift+z",
  zoomIn: "meta+=",
  zoomOut: "meta+-",
  fitPage: "meta+0",
  fitWidth: "meta+1",
  actual: "meta+2",
  toggleView: "meta+g",
  togglePane: "meta+shift+p",
} as const;

export function matchesShortcut(e: KeyboardEvent, shortcut: string): boolean {
  const parts = shortcut.split("+");
  const modMeta = parts.includes("meta") === (e.metaKey || e.ctrlKey);
  const modShift = parts.includes("shift") === e.shiftKey;
  const last = parts[parts.length - 1];
  const keyOk =
    last === "=" ? e.key === "=" && e.shiftKey : e.key.toLowerCase() === last.toLowerCase();
  return modMeta && modShift && keyOk;
}