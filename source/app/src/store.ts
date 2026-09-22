//! Tiny app store + CommandBus around the adapters.
//!
//! Undo/redo (A-c8): the Rust core mutates structure in place and does not yet
//! journal Operations, so exact multi-step undo is *not* available in this
//! scaffold. Stage 1 semantics here are honest and exact where cheap:
//!   - rotate: re-apply the inverse rotation (exact inverse).
//!   - reorder: re-apply the captured `before` order (exact inverse).
//!   - set_metadata: restore the previous metadata snapshot (exact inverse).
//!   - delete/insert_blank/extract: re-open the source (source bytes are
//!     untouched by structure ops => exact reset to original, but drops any
//!     earlier rotations). 
//! The build AIs will implement the true OperationLog on the Rust command layer
//! (Stage 1.5) so sequential undo/redo across a mixed sequence is exact; see
//! docs/STAGE0_ENGINE_INTERFACES.md §4 `revert`.

import { useSyncExternalStore } from "react";
import type { Document, Operation, TextEditParams } from "./types";
import {
  commitAdapter,
  metadataAdapter,
  openDocumentAdapter,
  pageTreeAdapter,
} from "./adapters/structuralAdapter";
import { cmdPageOps, cmdRenderBytes } from "./ipc";

export interface AppState {
  doc: Document | null;
  currentPage: number;
  zoom: number;
  zoomMode: "fit-page" | "fit-width" | "actual" | "custom";
  viewMode: "single" | "spread" | "continuous";
  busy: boolean;
  lastError: Error | null;
  lastCommit: { target: string; sha: string; size: number } | null;
  statusMessage: string;
  metadataOpen: boolean;
  /** Text-edit mode: viewer overlays clickable text runs. */
  editMode: boolean;
  /** Left pane (thumbnails / bookmarks). */
  paneOpen: boolean;
  paneTab: "pages" | "bookmarks";
}

let state: AppState = {
  doc: null,
  currentPage: 1,
  zoom: 1,
  zoomMode: "fit-page",
  viewMode: "continuous",
  busy: false,
  lastError: null,
  lastCommit: null,
  statusMessage: "No document open",
  metadataOpen: false,
  editMode: false,
  paneOpen: true,
  paneTab: "pages",
};

const listeners = new Set<() => void>();

function set(partial: Partial<AppState>) {
  state = { ...state, ...partial };
  listeners.forEach((l) => l());
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
  const { cmdOpen } = await import("./ipc");
  const fresh = await cmdOpen(doc.path);
  return {
    ...doc,
    pages: fresh.pages.map((p) => ({ ...p })),
    metadata: fresh.metadata,
  };
}

export const store = {
  async open(path: string) {
    set({ busy: true, lastError: null });
    try {
      const doc = await openDocumentAdapter.open(path);
      set({ doc, currentPage: 1, statusMessage: `Opened ${doc.pages.length} pages` });
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Open failed" });
    } finally {
      set({ busy: false });
    }
  },

  /** Run a PageTreeAdapter mutation and record its Operation. */
  async command(op: Operation, mutation: (doc: Document) => Promise<unknown>): Promise<boolean> {
    set({ busy: true, lastError: null });
    try {
      const doc = state.doc;
      if (!doc) throw new Error("no document open");
      await mutation(doc);
      let next = await refreshPages(doc);
      if (!next.passwordRequired && !next.encrypted) {
        try {
          const rb = await cmdRenderBytes();
          next = { ...next, bytesB64: rb.bytes_b64 };
        } catch {
          // Best-effort: keep the original opened bytes if serialization fails.
        }
      }
      set({
        doc: { ...next, history: [...next.history, op], redo: [] },
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

  async extract(pages: number[]) {
    const out = await pageTreeAdapter.extract(
      state.doc!,
      pages,
    );
    return out;
  },

  /** Undo — exact inverses where possible; see module doc. */
  async undo(): Promise<boolean> {
    const doc = state.doc;
    const op = doc?.history.at(-1);
    if (!doc || !op) return false;
    set({ busy: true, lastError: null });
    try {
      if (op.kind === "rotate") {
        const d = op.data as { pages: number[]; cw: boolean };
        await cmdPageOps("rotate", { pages: d.pages, cw: !d.cw });
      } else if (op.kind === "reorder") {
        await cmdPageOps("reorder", { new_order: op.before });
      } else if (op.kind === "delete") {
        // Exact reset to the original source (structural ops never wrote disk).
        const fresh = await openDocumentAdapter.open(doc.path);
        const next = { ...fresh, history: doc.history.slice(0, -1), redo: [op] };
        set({ doc: next, statusMessage: "Undid delete (source reset)" });
        return true;
      } else if (op.kind === "set_metadata") {
        await metadataAdapter.set(doc, op.beforeMeta?? {});
        // handled by refresh below
      } else {
        const fresh = await openDocumentAdapter.open(doc.path);
        const next = { ...fresh, history: doc.history.slice(0, -1), redo: [op] };
        set({ doc: next, statusMessage: `Undid ${op.kind} (source reset)` });
        return true;
      }
      const next = await refreshPages(doc);
      set({
        doc: { ...next, history: doc.history.slice(0, -1), redo: [op] },
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

  redo(): boolean {
    // Replay of a captured Operation payload; wired here for the AIs to fill
    // out with the Rust op-log (mirror of undo).
    return false;
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
    const prev = doc.metadata;
    await metadataAdapter.set(doc, updates);
    const next = await refreshPages(doc);
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