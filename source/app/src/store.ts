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
import type { CommitResult, Document, EditResult, InPlaceEdit, Operation, SearchResult } from "./types";
import {
  commitAdapter,
  documentFromOpen,
  metadataAdapter,
  openDocumentAdapter,
  pageTreeAdapter,
} from "./adapters/structuralAdapter";
import type { RenderSession } from "./adapters/renderAdapter";
import { unbind } from "./adapters/renderAdapter";
import {
  cmdAdoptPath,
  cmdCombine,
  cmdEditText,
  cmdExportPages,
  cmdNew,
  cmdOpen,
  cmdQuit,
  cmdRedo,
  cmdRenderBytes,
  cmdSave,
  cmdSetCrop,
  cmdSplit,
  cmdUndo,
  cmdUnlock,
  cmdWatermark,
  cmdAddAnnotation,
  cmdDeleteAnnotation,
  cmdMoveAnnotation,
  cmdRedact,
  cmdObjectOp,
  cmdInsertShape,
  cmdSetField,
  cmdInsertImage,
  cmdTabNew,
  cmdTabSwitch,
  cmdTabClose,
  cmdMovePagesToTab,
  cmdRestoreRecovery,
  cmdClearRecovery,
  cmdOcrRecognize,
  cmdOcrApply,
  type OcrLine,
} from "./ipc";
import { pageImage, toBase64 } from "./export";
import { loadSettings } from "./settings";
import { ask } from "@tauri-apps/plugin-dialog";
import { search as searchText } from "./adapters/renderAdapter";

/** Virtual path prefix the backend gives never-saved documents. */
export const UNTITLED = "stitchup-untitled:";
export const isUntitled = (doc: Document | null) => !!doc && doc.path.startsWith(UNTITLED);
/** Friendly names for untitled documents (e.g. recovered after a crash). */
const displayNames = new Map<string, string>();

export function docName(doc: Document | null): string {
  if (!doc) return "";
  const shown = displayNames.get(doc.path);
  if (shown) return shown;
  if (isUntitled(doc)) return `Untitled ${doc.path.slice(UNTITLED.length)}`;
  return doc.path.split(/[\\/]/).pop() ?? doc.path;
}

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
  /**
   * Shared pdf.js binding for the open document, owned here so both the main
   * viewer and the sidebar thumbnails render from ONE session (bind once).
   */
  session: RenderSession | null;
  /** CSS size of the scrolling viewer area (for fit-page/fit-width). */
  viewport: { width: number; height: number };
  /** Active search hits to highlight in the viewer (A-c9). */
  searchHits: SearchResult[];
  /** Index of the current hit within `searchHits` (-1 when none). */
  searchIndex: number;
  /** Bumped when the viewer should scroll `currentPage` into view. */
  scrollSeq: number;
  /** Which secondary dialog is open. */
  dialog: Dialog;
  /** Pages selected in the sidebar (empty = just the current page). */
  selection: number[];
  /** `doc.history.length` when last saved/opened (dirty when different). */
  savedLen: number;
  /** Current search text (Find, ⌘F / F3). */
  query: string;
  /** Ribbon tab shown (also switched from the Tabs menu). */
  ribbonTab: RibbonTabName;
  /** Armed drawing / selection tool (null = normal reading + text select). */
  tool: Tool | null;
  /** Label used by the Stamp tool. */
  stampLabel: string;
  /** Windows "Fore" colour: text, outlines, pen, underline. */
  foreColor: string;
  /** Windows "Back" colour: fill of new shapes (null = no fill). */
  backColor: string | null;
  strokeWidth: number;
  /** Select tool: picked content objects ("t12"/"i30") and annotations (obj #). */
  picked: { page: number; objects: string[]; annots: number[] } | null;
  /** Pending input for a placed annotation (note/text box/link/stamp/sign). */
  pendingInput: PendingInput | null;
  /** Image waiting to be placed by the Image tool. */
  pendingImage: PendingImage | null;
  /** Open document tabs (the active one's live state is the rest of AppState). */
  tabs: TabInfo[];
  activeTab: number;
}

export interface TabInfo {
  id: number;
  name: string;
  path: string | null;
  dirty: boolean;
}

/** Per-tab view state, parked while another tab is active. */
interface TabSlice {
  doc: Document | null;
  currentPage: number;
  zoom: number;
  zoomMode: AppState["zoomMode"];
  viewMode: AppState["viewMode"];
  savedLen: number;
  selection: number[];
  query: string;
  lastCommit: AppState["lastCommit"];
  statusMessage: string;
  /** The backend changed this tab's document while it was parked. */
  stale?: boolean;
}

const parked = new Map<number, TabSlice>();

export interface PendingImage {
  /** "jpeg" (original bytes) or "rgba" (decoded pixels). */
  format: "jpeg" | "rgba";
  b64: string;
  width: number;
  height: number;
  name: string;
}

export type Tool =
  | "highlight"
  | "underline"
  | "strikeout"
  | "note"
  | "textbox"
  | "rect"
  | "oval"
  | "line"
  | "arrow"
  | "pen"
  | "stamp"
  | "link"
  | "redact"
  | "select"
  | "shape-rect"
  | "shape-oval"
  | "shape-line"
  | "shape-arrow"
  | "sign"
  | "image";

export interface PendingInput {
  kind: "note" | "textbox" | "link" | "sign";
  page: number;
  /** PDF user-space rect (or point as a zero-size rect). */
  rect: [number, number, number, number];
}

export type RibbonTabName = "Document" | "Pages" | "Annotate" | "Content" | "Zoom" | "Navigate" | "Security";
export const RIBBON_TABS: RibbonTabName[] = ["Document", "Pages", "Annotate", "Content", "Zoom", "Navigate", "Security"];

export type Dialog =
  | null
  | "about"
  | "extract"
  | "split"
  | "new"
  | "addPage"
  | "watermark"
  | "encrypt"
  | "attachments"
  | "exportImages"
  | "print"
  | "help"
  | "sign"
  | "recolor"
  | "font"
  | "settings"
  | "recovery";

let state: AppState = {
  doc: null,
  currentPage: 1,
  zoom: 1,
  // Documents open fit-to-width, as in the Windows build.
  zoomMode: "fit-width",
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
  session: null,
  viewport: { width: 0, height: 0 },
  searchHits: [],
  searchIndex: -1,
  scrollSeq: 0,
  dialog: null,
  selection: [],
  savedLen: 0,
  query: "",
  ribbonTab: "Document",
  tool: null,
  stampLabel: "Approved",
  foreColor: "#d0021b",
  backColor: null,
  strokeWidth: 2,
  picked: null,
  pendingInput: null,
  pendingImage: null,
  tabs: [{ id: 1, name: "New Tab", path: null, dirty: false }],
  activeTab: 1,
};

function sliceOf(s: AppState): TabSlice {
  return {
    doc: s.doc,
    currentPage: s.currentPage,
    zoom: s.zoom,
    zoomMode: s.zoomMode,
    viewMode: s.viewMode,
    savedLen: s.savedLen,
    selection: s.selection,
    query: s.query,
    lastCommit: s.lastCommit,
    statusMessage: s.statusMessage,
  };
}

const EMPTY_SLICE: TabSlice = {
  doc: null,
  currentPage: 1,
  zoom: 1,
  zoomMode: "fit-width",
  viewMode: "continuous",
  savedLen: 0,
  selection: [],
  query: "",
  lastCommit: null,
  statusMessage: "No document open",
};

/** "#rrggbb" → [r, g, b] in 0..1. */
export function rgb(hex: string): [number, number, number] {
  const n = parseInt(hex.replace("#", ""), 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

const listeners = new Set<() => void>();

let noticeTimer: ReturnType<typeof setTimeout> | null = null;

function set(partial: Partial<AppState>) {
  state = { ...state, ...partial };
  // The active tab's label follows its document.
  const t = state.tabs.find((x) => x.id === state.activeTab);
  if (t) {
    const d = state.doc;
    const name = d ? docName(d) : "New Tab";
    const dirty = !!d && (d.history.length !== state.savedLen || (isUntitled(d) && d.history.length > 0));
    const path = d && !isUntitled(d) ? d.path : null;
    if (t.name !== name || t.dirty !== dirty || t.path !== path) {
      state = { ...state, tabs: state.tabs.map((x) => (x.id === t.id ? { ...x, name, dirty, path } : x)) };
    }
  }
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
  if (doc.passwordRequired) return doc;
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
    // Already open in a tab → just show it.
    const existing = state.tabs.find((t) => t.path === path);
    if (existing) {
      await this.switchTab(existing.id);
      return;
    }
    await this.freshTab();
    set({ busy: true, lastError: null });
    try {
      const doc = await openDocumentAdapter.open(path);
      set({
        doc,
        currentPage: 1,
        zoomMode: loadSettings().openZoom === "actual" ? "actual" : loadSettings().openZoom,
        zoom: loadSettings().openZoom === "actual" ? 1 : state.zoom,
        paneOpen: loadSettings().sidebarOnOpen,
        scrollSeq: state.scrollSeq + 1,
        selection: [],
        savedLen: 0,
        editMode: false,
        selectionText: "",
        searchHits: [],
        searchIndex: -1,
        unlockOpen: doc.passwordRequired,
        statusMessage: doc.passwordRequired
          ? "This document is password-protected"
          : `Opened ${doc.pages.length} page${doc.pages.length === 1 ? "" : "s"}`,
      });
      const w = doc.warnings.filter((x) => !/password-protected/.test(x));
      if (!doc.passwordRequired && w.length) {
        notify(`⚠ ${w.length} warning${w.length > 1 ? "s" : ""}: ${w[0]}`);
      }
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
      notify("Unlocked — copies you save will not be password-protected");
      return true;
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Unlock failed" });
      return false;
    } finally {
      set({ busy: false });
    }
  },

  /** Start a new blank document (Letter, A4, ...). */
  async newDocument(widthPt: number, heightPt: number) {
    await this.freshTab();
    return this.adopt(() => cmdNew(widthPt, heightPt), "New document");
  },

  /** Combine several PDFs into a new untitled document. */
  async combine(paths: string[]) {
    await this.freshTab();
    return this.adopt(() => cmdCombine(paths), `Combined ${paths.length} files`);
  },

  /** Replace the open document with one the backend just built. */
  async adopt(make: () => Promise<Parameters<typeof documentFromOpen>[0]>, msg: string) {
    set({ busy: true, lastError: null });
    try {
      const doc = documentFromOpen(await make());
      set({
        doc,
        currentPage: 1,
        zoomMode: loadSettings().openZoom === "actual" ? "actual" : loadSettings().openZoom,
        zoom: loadSettings().openZoom === "actual" ? 1 : state.zoom,
        paneOpen: loadSettings().sidebarOnOpen,
        scrollSeq: state.scrollSeq + 1,
        selection: [],
        savedLen: 0,
        editMode: false,
        searchHits: [],
        searchIndex: -1,
        statusMessage: msg,
      });
      return true;
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Failed" });
      return false;
    } finally {
      set({ busy: false });
    }
  },

  /** True when there are changes since the last save/open. */
  isDirty(): boolean {
    const d = state.doc;
    return !!d && (d.history.length !== state.savedLen || (isUntitled(d) && d.history.length > 0));
  },

  /** Save in place (atomic). Untitled documents return false → use Save As. */
  async save(): Promise<boolean> {
    const doc = state.doc;
    if (!doc || doc.passwordRequired) return false;
    if (isUntitled(doc)) return false;
    set({ busy: true, lastError: null });
    try {
      const res = await cmdSave();
      set({
        savedLen: doc.history.length,
        lastCommit: { target: res.published_path, sha: res.new_sha256, size: res.size_bytes },
        statusMessage: `Saved ${docName(doc)}`,
      });
      notify(`Saved ${docName(doc)}`);
      return true;
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Save failed" });
      return false;
    } finally {
      set({ busy: false });
    }
  },

  /** After Save As, the new file becomes the document (later Saves go there). */
  async adoptSavedPath(path: string) {
    const doc = state.doc;
    if (!doc) return;
    try {
      const next = documentFromOpen(await cmdAdoptPath(path));
      set({
        doc: { ...next, bookmarks: doc.bookmarks, history: doc.history, redo: doc.redo },
        savedLen: doc.history.length,
      });
    } catch (e) {
      set({ lastError: e as Error });
    }
  },

  /** Text watermark on all (or the given) pages. */
  async watermark(spec: { text: string; size: number; placement: string; opacity: number; color: number[]; pages?: number[] }) {
    const doc = state.doc;
    if (!doc) return false;
    return this.command(
      { kind: "watermark", before: doc.pages.map((p) => p.page_no), data: spec },
      () => cmdWatermark(spec),
      () => ({ status: `Watermark added to ${spec.pages?.length || doc.pages.length} page(s)` }),
    );
  },

  /** Set crop boxes `[page, x0, y0, x1, y1]` (PDF user space). */
  async crop(boxes: number[][]) {
    const doc = state.doc;
    if (!doc || boxes.length === 0) return false;
    return this.command(
      { kind: "crop", before: doc.pages.map((p) => p.page_no), data: boxes },
      () => cmdSetCrop(boxes),
      () => ({ status: `Cropped ${boxes.length} page(s)` }),
    );
  },

  /** Import (append) another PDF and jump to its first page. */
  async importPdf(path: string) {
    const before = state.doc?.pages.length ?? 0;
    const ok = await this.merge(path);
    if (ok) this.goToPage(before + 1);
    return ok;
  },

  /** Pages the page commands act on: the sidebar selection or the current page. */
  targets(): number[] {
    return state.selection.length ? state.selection : [state.currentPage];
  },

  setPageSelection(pages: number[]) {
    set({ selection: pages });
  },

  selectAllPages() {
    const doc = state.doc;
    if (doc) set({ selection: doc.pages.map((p) => p.page_no) });
  },

  setRibbonTab(tab: RibbonTabName) {
    set({ ribbonTab: tab });
  },

  // ---- Find
  /** Typing a new search term drops the old matches (Enter searches again). */
  setQuery(q: string) {
    if (q === state.query) return;
    set({ query: q, searchHits: [], searchIndex: -1 });
  },

  async find(q = state.query): Promise<number> {
    const session = state.session;
    const doc = state.doc;
    if (!doc || !session || !q.trim()) return 0;
    const hits = await searchText(session, q.trim(), {}, [1, doc.pages.length]);
    // Start from the first hit at or after the current page.
    const start = Math.max(0, hits.findIndex((h) => h.page_no >= state.currentPage));
    set({ query: q, searchHits: hits, searchIndex: hits.length ? start : -1 });
    if (hits.length) this.goToPage(hits[start].page_no);
    else notify(`No matches for “${q.trim()}”`);
    return hits.length;
  },

  /** F3 / ⇧F3: next / previous match, wrapping around. */
  async findNext(step: 1 | -1 = 1) {
    if (!state.searchHits.length) {
      if (state.query) await this.find();
      return;
    }
    const n = (state.searchIndex + step + state.searchHits.length) % state.searchHits.length;
    set({ searchIndex: n });
    this.goToPage(state.searchHits[n].page_no);
  },

  // ---- tools & annotations
  /** Arm a tool (same tool again = disarm). Leaving clears the selection. */
  setTool(tool: Tool | null) {
    const next = state.tool === tool ? null : tool;
    set({ tool: next, picked: null, editMode: false, pendingInput: null });
    if (next === "select") notify("Select: click an object or drag a box · drag to move · ⌫ deletes");
  },

  setStampLabel(label: string) {
    set({ stampLabel: label });
  },

  setColors(c: { fore?: string; back?: string | null; width?: number }) {
    set({
      foreColor: c.fore ?? state.foreColor,
      backColor: c.back === undefined ? state.backColor : c.back,
      strokeWidth: c.width ?? state.strokeWidth,
    });
  },

  setPicked(picked: AppState["picked"]) {
    set({ picked });
  },

  setPendingInput(p: PendingInput | null) {
    set({ pendingInput: p });
  },

  /** Arm the Image tool with a decoded image. */
  armImage(img: PendingImage) {
    set({ pendingImage: img, tool: "image", picked: null, editMode: false });
    notify(`Click or drag on the page to place ${img.name}`);
  },

  async placeImage(page: number, rect: number[]) {
    const img = state.pendingImage;
    const doc = state.doc;
    if (!img || !doc) return false;
    const ok = await this.command(
      { kind: "object", before: doc.pages.map((p) => p.page_no), data: { image: img.name } },
      () => cmdInsertImage(page, rect, { format: img.format, b64: img.b64, width: img.width, height: img.height }),
      () => ({ status: `Inserted image ${img.name} on page ${page}`, page }),
    );
    set({ tool: null, pendingImage: null });
    return ok;
  },

  /**
   * OCR: recognise text on scanned pages (Apple Vision, on this Mac) and add
   * it as an invisible layer — the page looks the same but becomes
   * searchable and copyable. `scope`: the current page, pages with no text
   * yet, or every page. One undo step.
   */
  async recognizeText(scope: "current" | "untexted" | "all") {
    const doc = state.doc;
    const session = state.session;
    if (!doc || !session) return false;
    let pages = scope === "current" ? [state.currentPage] : doc.pages.map((p) => p.page_no);
    if (scope === "untexted") {
      const bare: number[] = [];
      for (const n of pages) {
        const tc = await (await session.pdf.getPage(n)).getTextContent();
        if (!tc.items.some((i) => "str" in i && i.str.trim())) bare.push(n);
      }
      if (!bare.length) {
        this.notify("Every page already has text — choose This Page or All Pages to recognise anyway");
        return false;
      }
      pages = bare;
    }
    const found: { page: number; rotation: number; lines: OcrLine[] }[] = [];
    set({ busy: true });
    try {
      for (const [i, n] of pages.entries()) {
        this.notify(`Recognising text: page ${i + 1} of ${pages.length}…`);
        const rotation = doc.pages.find((p) => p.page_no === n)?.rotation ?? 0;
        const png = await pageImage(session, n, rotation, 300, "image/png");
        const lines = await cmdOcrRecognize(toBase64(png));
        if (lines.length) found.push({ page: n, rotation, lines });
      }
    } catch (e) {
      set({ busy: false });
      this.notify(`Text recognition failed: ${(e as { message?: string }).message ?? e}`);
      return false;
    }
    set({ busy: false });
    const total = found.reduce((a, p) => a + p.lines.length, 0);
    if (!total) {
      this.notify(`No text found on ${pages.length === 1 ? "this page" : `${pages.length} pages`}`);
      return false;
    }
    return this.command(
      { kind: "object", before: doc.pages.map((p) => p.page_no), data: { ocr: found.length } },
      () => cmdOcrApply(found),
      () => ({ status: `Recognised ${total} line${total === 1 ? "" : "s"} on ${found.length} page${found.length === 1 ? "" : "s"} — now searchable`, page: found[0].page }),
    );
  },

  /** Fill a form field (text/choice: string; checkbox: boolean; radio: option). */
  async setField(name: string, value: string | boolean) {
    const doc = state.doc;
    if (!doc) return false;
    return this.command(
      { kind: "object", before: doc.pages.map((p) => p.page_no), data: { name, value } },
      () => cmdSetField(name, value),
      () => ({ status: `Filled “${name}”` }),
    );
  },

  /** Add an annotation (spec as in stitchup_core::annots::AnnotSpec). */
  async annotate(page: number, spec: { kind: string } & Record<string, unknown>) {
    const doc = state.doc;
    if (!doc) return false;
    return this.command(
      { kind: "annotate", before: doc.pages.map((p) => p.page_no), data: spec },
      () => cmdAddAnnotation(page, spec),
      () => ({ status: `Added ${spec.kind.replace("_", " ")} on page ${page}`, page }),
    );
  },

  async redact(page: number, rect: number[]) {
    const doc = state.doc;
    if (!doc) return false;
    let msg = "Redacted";
    return this.command(
      { kind: "redact", before: doc.pages.map((p) => p.page_no), data: rect },
      async () => {
        const r = await cmdRedact(page, rect);
        const rep = r.report;
        const parts = [];
        if (rep.glyphs_removed) parts.push(`${rep.glyphs_removed} characters`);
        if (rep.images_removed) parts.push(`${rep.images_removed} image${rep.images_removed > 1 ? "s" : ""}`);
        if (rep.annotations_removed) parts.push(`${rep.annotations_removed} annotation${rep.annotations_removed > 1 ? "s" : ""}`);
        msg = `Redacted: removed ${parts.length ? parts.join(", ") : "nothing under the box (area blacked out)"}`;
        if (rep.images_covered) msg += ` · ${rep.images_covered} image(s) partly under the box were covered, not removed`;
        notify(msg);
        return r;
      },
      () => ({ status: msg, page }),
    );
  },

  /** Delete what the Select tool picked (content objects and annotations). */
  async deletePicked(): Promise<boolean> {
    const p = state.picked;
    const doc = state.doc;
    if (!p || !doc || (!p.objects.length && !p.annots.length)) return false;
    const ok = await this.command(
      { kind: "object", before: doc.pages.map((x) => x.page_no), data: p },
      async () => {
        if (p.objects.length) await cmdObjectOp(p.page, { op: "delete", ids: p.objects });
        // Annotation numbers are stable across deletes (they are object ids).
        for (const a of p.annots) await cmdDeleteAnnotation(p.page, a);
      },
      () => ({ status: `Deleted ${p.objects.length + p.annots.length} object(s)`, page: p.page }),
    );
    set({ picked: null });
    return ok;
  },

  /** Move picked objects/annotations by (dx, dy) PDF points. */
  async movePicked(dx: number, dy: number): Promise<boolean> {
    const p = state.picked;
    const doc = state.doc;
    if (!p || !doc) return false;
    const ok = await this.command(
      { kind: "object", before: doc.pages.map((x) => x.page_no), data: { ...p, dx, dy } },
      async () => {
        if (p.objects.length) await cmdObjectOp(p.page, { op: "move", ids: p.objects, dx, dy });
        for (const a of p.annots) await cmdMoveAnnotation(p.page, a, dx, dy);
      },
      () => ({ status: `Moved ${p.objects.length + p.annots.length} object(s)`, page: p.page }),
    );
    set({ picked: null });
    return ok;
  },

  async objectOp(data: { op: "recolor" | "font"; color?: number[]; family?: string; size?: number }) {
    const p = state.picked;
    const doc = state.doc;
    if (!p || !doc || !p.objects.length) {
      notify("Select text first (Content ▸ Select, then click a line)");
      return false;
    }
    const ok = await this.command(
      { kind: "object", before: doc.pages.map((x) => x.page_no), data },
      () => cmdObjectOp(p.page, { ...data, ids: p.objects }),
      () => ({ status: data.op === "recolor" ? "Recoloured" : "Font changed", page: p.page }),
    );
    set({ picked: null });
    return ok;
  },

  async insertShape(page: number, spec: Record<string, unknown>) {
    const doc = state.doc;
    if (!doc) return false;
    return this.command(
      { kind: "object", before: doc.pages.map((p) => p.page_no), data: spec },
      () => cmdInsertShape(page, spec),
      () => ({ status: `Inserted ${spec.kind} on page ${page}`, page }),
    );
  },

  // ---- document tabs
  /** Use the current tab if it is empty, otherwise open a new one. */
  async freshTab() {
    if (state.doc) await this.newTab();
  },

  async newTab(): Promise<number> {
    let id: number;
    try {
      id = await cmdTabNew();
    } catch {
      return state.activeTab;
    }
    parked.set(state.activeTab, sliceOf(state));
    set({
      ...EMPTY_SLICE,
      tabs: [...state.tabs, { id, name: "New Tab", path: null, dirty: false }],
      activeTab: id,
      session: null,
      searchHits: [],
      searchIndex: -1,
      picked: null,
      tool: null,
      editMode: false,
      scrollSeq: state.scrollSeq + 1,
    });
    return id;
  },

  async switchTab(id: number) {
    if (id === state.activeTab || !state.tabs.some((t) => t.id === id)) return;
    await cmdTabSwitch(id);
    parked.set(state.activeTab, sliceOf(state));
    const slice = parked.get(id) ?? EMPTY_SLICE;
    parked.delete(id);
    set({ ...slice, stale: undefined, activeTab: id, session: null, searchHits: [], searchIndex: -1, picked: null, tool: null, editMode: false, scrollSeq: state.scrollSeq + 1 } as Partial<AppState>);
    if (slice.stale && slice.doc) {
      // Pages arrived from another tab while this one was parked.
      const next = await withFreshBytes(await refreshPages(slice.doc));
      set({ doc: { ...next, history: [...slice.doc.history, { kind: "merge", before: [], data: "from tab" }], redo: [] } });
    }
  },

  async closeTab(id: number) {
    const t = state.tabs.find((x) => x.id === id);
    if (!t) return;
    if (t.dirty) {
      let ok = false;
      try {
        ok = await ask(`“${t.name}” has unsaved changes. Close it anyway?`, { title: "Close tab", kind: "warning", okLabel: "Close", cancelLabel: "Cancel" });
      } catch {
        ok = window.confirm(`“${t.name}” has unsaved changes. Close it anyway?`);
      }
      if (!ok) return;
    }
    const idx = state.tabs.findIndex((x) => x.id === id);
    if (id === state.activeTab) {
      const other = state.tabs[idx + 1] ?? state.tabs[idx - 1];
      if (other) {
        await this.switchTab(other.id);
      } else {
        // Last tab: keep an empty one.
        await cmdTabClose(id).catch(() => {});
        set({ ...EMPTY_SLICE, session: null, searchHits: [], searchIndex: -1, picked: null, tool: null });
        return;
      }
    }
    await cmdTabClose(id).catch(() => {});
    parked.delete(id);
    set({ tabs: state.tabs.filter((x) => x.id !== id) });
  },

  async cycleTab(step: 1 | -1) {
    const i = state.tabs.findIndex((t) => t.id === state.activeTab);
    const next = state.tabs[(i + step + state.tabs.length) % state.tabs.length];
    if (next) await this.switchTab(next.id);
  },

  /** Move pages of the active document to the end of another tab's document. */
  async movePagesToTab(pages: number[], to: number) {
    const doc = state.doc;
    const target = state.tabs.find((t) => t.id === to);
    if (!doc || !target) return false;
    const ok = await this.command(
      { kind: "delete", before: doc.pages.map((p) => p.page_no), data: { pages, to } },
      () => cmdMovePagesToTab(pages, to),
      () => ({ status: `Moved ${pages.length} page${pages.length > 1 ? "s" : ""} to “${target.name}”` }),
    );
    if (ok) {
      const slice = parked.get(to);
      if (slice) parked.set(to, { ...slice, stale: true });
      state = { ...state, tabs: state.tabs.map((t) => (t.id === to ? { ...t, dirty: true } : t)) };
      listeners.forEach((l) => l());
    }
    return ok;
  },

  /** Any tab (active or parked) with unsaved changes? */
  anyDirty(): boolean {
    return state.tabs.some((t) => t.dirty);
  },

  setUnlockOpen(open: boolean) {
    set({ unlockOpen: open });
  },

  /** Run a PageTreeAdapter mutation and record its Operation. */
  async command(
    op: Operation,
    mutation: (doc: Document) => Promise<unknown>,
    after?: (result: unknown) => { status?: string; page?: number },
  ): Promise<boolean> {
    set({ busy: true, lastError: null });
    try {
      const doc = state.doc;
      if (!doc) throw new Error("no document open");
      const result = await mutation(doc);
      let next = await refreshPages(doc);
      next = await withFreshBytes(next);
      const extra = after?.(result) ?? {};
      set({
        doc: { ...next, history: [...next.history, op], redo: [] },
        currentPage: clampPage(next, extra.page ?? state.currentPage),
        // Text/page positions changed: stale search boxes would mislead.
        searchHits: [],
        searchIndex: -1,
        selection: [],
        statusMessage: extra.status ?? `Applied ${LABEL[op.kind] ?? op.kind}`,
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

  /**
   * Insert a blank page after `afterPage` (0 = at the start). The size
   * defaults to that page as displayed (rotation-swapped), so a blank page
   * inserted into an A5 landscape document is A5 landscape too (A-c14).
   */
  async insertBlank(afterPage: number, widthPt?: number, heightPt?: number) {
    const doc = state.doc;
    if (!doc) return false;
    const ref = doc.pages.find((p) => p.page_no === Math.max(1, afterPage)) ?? doc.pages[0];
    const swap = ref ? ((ref.rotation % 180) + 180) % 180 === 90 : false;
    const w = widthPt ?? (ref ? (swap ? ref.height_pt : ref.width_pt) : 612);
    const h = heightPt ?? (ref ? (swap ? ref.width_pt : ref.height_pt) : 792);
    const before = doc.pages.map((p) => p.page_no);
    return this.command(
      { kind: "insert_blank", before, data: { after: afterPage, width_pt: w, height_pt: h } },
      () => pageTreeAdapter.insertBlank(doc, afterPage, w, h),
      () => ({ status: `Inserted a blank page after page ${afterPage}`, page: afterPage + 1 }),
    );
  },

  /** Live in-place text edit (see stitchup_core::textedit). */
  async editText(params: InPlaceEdit): Promise<EditResult | null> {
    const doc = state.doc;
    if (!doc || doc.passwordRequired) return null;
    const before = doc.pages.map((p) => p.page_no);
    let res: EditResult | null = null;
    const ok = await this.command(
      { kind: "edit_text", before, data: params },
      async () => (res = await cmdEditText(params)),
      () => {
        const r = res as EditResult | null;
        const msg =
          r?.method === "in_place"
            ? `Text edited (${r.note})`
            : r
              ? `Text edited — ${r.note}`
              : "Text edited";
        notify(msg);
        return { status: msg };
      },
    );
    return ok ? res : null;
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

  /** Save `pages` (1-based) as a new PDF; the open document is unchanged. */
  async exportPages(pages: number[], target: string): Promise<CommitResult | null> {
    set({ busy: true, lastError: null });
    try {
      const res = await cmdExportPages(pages, target, true);
      const msg = `Saved ${pages.length} page${pages.length === 1 ? "" : "s"} to ${res.published_path}`;
      set({ statusMessage: msg });
      notify(msg);
      return res;
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Extract failed" });
      return null;
    } finally {
      set({ busy: false });
    }
  },

  /** Split into files of `every` pages inside `dir`. */
  async split(every: number, dir: string, stem: string): Promise<CommitResult[] | null> {
    set({ busy: true, lastError: null });
    try {
      const res = await cmdSplit(every, dir, stem, false);
      const msg = `Split into ${res.length} files in ${dir}`;
      set({ statusMessage: msg });
      notify(msg);
      return res;
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Split failed" });
      return null;
    } finally {
      set({ busy: false });
    }
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
        searchHits: [],
        searchIndex: -1,
        statusMessage: `Undid ${LABEL[op.kind] ?? op.kind}`,
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
        searchHits: [],
        searchIndex: -1,
        statusMessage: `Redid ${LABEL[op.kind] ?? op.kind}`,
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
      notify(`Saved to ${res.published_path}`);
      return res;
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Save failed" });
      throw e;
    } finally {
      set({ busy: false });
    }
  },

  async updateMetadata(updates: Parameters<typeof metadataAdapter.set>[1]): Promise<boolean> {
    const doc = state.doc;
    if (!doc) return false;
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
      return true;
    } catch (e) {
      set({ lastError: e as Error, statusMessage: "Metadata update failed" });
      return false;
    } finally {
      set({ busy: false });
    }
  },

  setZoom(z: number, mode: AppState["zoomMode"]) {
    set({ zoom: z, zoomMode: mode });
  },

  /**
   * Switch to a fit mode; the exact scale is computed by Viewer (which owns the
   * viewport measurement) on the next render via applyFit().
   */
  setFitMode(mode: "fit-page" | "fit-width") {
    set({ zoomMode: mode });
  },

  /**
   * Called by the Viewer with the measured viewport and the current page's
   * visible size so a fit mode produces an exact scale. No-op in custom/actual.
   */
  applyFit(viewport: { width: number; height: number }, page: { width: number; height: number }) {
    const mode = state.zoomMode;
    if (mode !== "fit-page" && mode !== "fit-width") return;
    const pad = 18; // must match .viewer-scroll padding
    const availW = Math.max(1, viewport.width - pad * 2);
    const availH = Math.max(1, viewport.height - pad * 2);
    const w = Math.max(1, page.width);
    const h = Math.max(1, page.height);
    let z = mode === "fit-width" ? availW / w : Math.min(availW / w, availH / h);
    z = Math.max(0.25, Math.min(5, z));
    if (Math.abs(z - state.zoom) < 0.001) return;
    set({ zoom: z });
  },

  /** Record the scrolling viewport size (drives fit-page/fit-width). */
  setViewport(width: number, height: number) {
    const p = state.viewport;
    if (p.width === width && p.height === height) return;
    set({ viewport: { width, height } });
  },

  /** Publish search hits + the current index so the viewer highlights them. */
  setSearch(hits: SearchResult[], index: number) {
    set({ searchHits: hits, searchIndex: index });
  },

  /** Move to a specific search hit index (wraps handled by the caller). */
  setSearchIndex(index: number) {
    set({ searchIndex: index });
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

  /** Add/remove a bookmark for a specific page (sidebar star toggle). */
  toggleBookmark(n: number) {
    const doc = state.doc;
    if (!doc) return;
    const has = doc.bookmarks.includes(n);
    const next: Document = {
      ...doc,
      bookmarks: has
        ? doc.bookmarks.filter((b) => b !== n)
        : [...doc.bookmarks, n].sort((a, b) => a - b),
    };
    set({ doc: next, statusMessage: has ? `Bookmark removed (page ${n})` : `Bookmarked page ${n}` });
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

  /** Jump to page `n` and scroll it into view. */
  goToPage(n: number) {
    const doc = state.doc;
    if (!doc || n < 1 || n > doc.pages.length) return;
    set({ currentPage: n, scrollSeq: state.scrollSeq + 1 });
  },

  /** The viewer reports the page under the viewport centre while scrolling. */
  setCurrentFromScroll(n: number) {
    if (n !== state.currentPage) set({ currentPage: n });
  },

  setDialog(dialog: AppState["dialog"]) {
    set({ dialog });
  },

  notify,

  setMetadataOpen(open: boolean) {
    set({ metadataOpen: open });
  },

  setEditMode(on: boolean) {
    set({ editMode: on });
  },

  /**
   * Swap the shared render session. Destroys the previous binding so a doc
   * change (or close) never leaks a pdf.js worker.
   */
  setSession(session: RenderSession | null) {
    const prev = state.session;
    if (prev && prev !== session) void unbind(prev);
    if (prev !== session) set({ session });
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

  /** Quit the app (⌘Q): asks first when any tab has unsaved changes. */
  async exitApp(): Promise<void> {
    if (!(await this.confirmDiscard("Quit Stitchup"))) return;
    await cmdClearRecovery().catch(() => {});
    await cmdQuit();
  },

  /** True when there is nothing unsaved, or the user agrees to lose it. */
  async confirmDiscard(title: string): Promise<boolean> {
    const dirty = state.tabs.filter((t) => t.dirty);
    if (!dirty.length) return true;
    const names = dirty.map((t) => `“${t.name}”`).join(", ");
    const msg = `${names} ${dirty.length > 1 ? "have" : "has"} unsaved changes. Discard them and quit?`;
    try {
      return await ask(msg, { title, kind: "warning", okLabel: "Discard and Quit", cancelLabel: "Cancel" });
    } catch {
      return window.confirm(msg);
    }
  },

  /** Re-open a crash-recovery snapshot in its own tab. */
  async restoreRecovered(file: string, name: string) {
    await this.freshTab();
    const label = `${name.replace(/\.pdf$/i, "")} (recovered).pdf`;
    const ok = await this.adopt(async () => {
      const payload = await cmdRestoreRecovery(file);
      displayNames.set(payload.path, label);
      return payload;
    }, `Recovered ${name} — use Save As to keep it`);
    return ok;
  },
};

/** Keyboard shortcut map (accessibility, A-c list). */
/** Friendly names for operations in status messages. */
const LABEL: Partial<Record<Operation["kind"], string>> = {
  reorder: "page reorder",
  rotate: "rotation",
  delete: "page delete",
  insert_blank: "blank page",
  edit_text: "text edit",
  set_metadata: "properties edit",
  merge: "merge",
  watermark: "watermark",
  crop: "crop",
  annotate: "annotation",
  redact: "redaction",
  object: "object edit",
};

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
  exit: "meta+q",
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