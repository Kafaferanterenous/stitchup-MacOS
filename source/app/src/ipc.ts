//! Thin, typed bridge to the Rust core (Tauri commands).
//! `invoke` failures from Tauri arrive as objects shaped like EngineError when
//! the Rust side rejects with `EngineError`; normalize anything else.

import { invoke } from "@tauri-apps/api/core";
import type { EngineError, OpenResult, CommitResult, PageMeta, Metadata, InPlaceEdit, EditResult } from "./types";
import { isEngineError } from "./types";

async function cmd<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(name, args);
  } catch (e) {
    if (isEngineError(e)) {
      throw e as EngineError;
    }
    const err: EngineError = { code: "open_failed", message: String(e), detail: e };
    throw err;
  }
}

export interface OpenPayload extends OpenResult {}

export async function cmdOpen(path: string): Promise<OpenPayload> {
  return cmd<OpenPayload>("open_document", { path });
}

export async function cmdPageOps(kind: string, data: unknown): Promise<PageMeta[]> {
  return cmd<PageMeta[]>("page_op", { kind, data });
}

export async function cmdSetMetadata(meta: Metadata): Promise<PageMeta[]> {
  return cmd<PageMeta[]>("cmd_set_metadata", { meta });
}

export async function cmdUndo(): Promise<PageMeta[]> {
  return cmd<PageMeta[]>("undo");
}

export async function cmdRedo(): Promise<PageMeta[]> {
  return cmd<PageMeta[]>("redo");
}

export async function cmdUnlock(password: string): Promise<OpenPayload> {
  return cmd<OpenPayload>("unlock_document", { password });
}

export async function cmdMerge(path: string): Promise<PageMeta[]> {
  return cmd<PageMeta[]>("merge_document", { path });
}

export async function cmdCommit(target: string, overwrite: boolean): Promise<CommitResult> {
  return cmd<CommitResult>("commit_export", { target, overwrite });
}

export async function cmdSourceHash(): Promise<{ source_hash: string }> {
  return cmd<{ source_hash: string }>("source_hash");
}

/** Serialize the CURRENT in-memory document so the viewer can rebind/re-render. */
export async function cmdRenderBytes(): Promise<{ bytes_b64: string }> {
  return cmd<{ bytes_b64: string }>("render_bytes");
}

/** Quit the app (File ▸ Exit). */
export async function cmdQuit(): Promise<void> {
  return cmd<void>("quit_app");
}
/** Live in-place text edit; reports how the edit was applied. */
export async function cmdEditText(data: InPlaceEdit): Promise<EditResult> {
  return cmd<EditResult>("edit_text", { data });
}

/** Save the given pages (1-based) as a new PDF (atomic). */
export async function cmdExportPages(pages: number[], target: string, overwrite: boolean): Promise<CommitResult> {
  return cmd<CommitResult>("export_pages", { pages, target, overwrite });
}

/** Split into files of `every` pages: <dir>/<stem>-partN.pdf. */
export async function cmdSplit(every: number, dir: string, stem: string, overwrite: boolean): Promise<CommitResult[]> {
  return cmd<CommitResult[]>("split_document", { every, dir, stem, overwrite });
}

// ---- document-level commands
export const cmdNew = (widthPt: number, heightPt: number) => cmd<OpenPayload>("new_document", { widthPt, heightPt });
export const cmdSave = () => cmd<CommitResult>("save_document");
export const cmdAdoptPath = (path: string) => cmd<OpenPayload>("adopt_saved_path", { path });
export const cmdCombine = (paths: string[]) => cmd<OpenPayload>("combine_documents", { paths });
export const cmdWatermark = (spec: unknown) => cmd<PageMeta[]>("add_watermark", { spec });
export const cmdSetCrop = (boxes: number[][]) => cmd<PageMeta[]>("set_crop", { boxes });
export const cmdSaveEncrypted = (target: string, userPassword: string, ownerPassword: string) =>
  cmd<CommitResult>("save_encrypted", { target, userPassword, ownerPassword });
export const cmdSaveCompressed = (target: string) =>
  cmd<{ before: number; after: number; result: CommitResult }>("save_compressed", { target });
export interface AttachmentInfo {
  index: number;
  name: string;
  size: number;
  mime: string;
  description: string;
}
export const cmdListAttachments = () => cmd<AttachmentInfo[]>("list_attachments");
export const cmdSaveAttachment = (index: number, target: string) => cmd<CommitResult>("save_attachment", { index, target });
export const cmdWriteFile = (path: string, b64: string) => cmd<CommitResult>("write_file", { path, b64 });
export const cmdTakePendingOpen = () => cmd<string[]>("take_pending_open");

// ---- annotations, objects, redaction, signatures
export const cmdAddAnnotation = (page: number, spec: unknown) => cmd<PageMeta[]>("add_annotation", { page, spec });
export const cmdDeleteAnnotation = (page: number, obj: number) => cmd<PageMeta[]>("delete_annotation", { page, obj });
export const cmdMoveAnnotation = (page: number, obj: number, dx: number, dy: number) => cmd<PageMeta[]>("move_annotation", { page, obj, dx, dy });
export interface RedactReport {
  glyphs_removed: number;
  images_removed: number;
  images_covered: number;
  annotations_removed: number;
}
export const cmdRedact = (page: number, rect: number[]) => cmd<{ pages: PageMeta[]; report: RedactReport }>("redact_area", { page, rect });
export interface PageObject {
  id: string;
  kind: "text" | "image" | "graphic";
  text: string;
  rect: [number, number, number, number];
}
export const cmdListObjects = (page: number) => cmd<PageObject[]>("list_objects", { page });
export const cmdObjectOp = (page: number, data: unknown) => cmd<PageMeta[]>("object_op", { page, data });
export const cmdInsertShape = (page: number, spec: unknown) => cmd<PageMeta[]>("insert_shape", { page, spec });
export const cmdSign = (target: string, spec: unknown) => cmd<CommitResult>("sign_document", { target, spec });
export const cmdOpenUrl = (url: string) => cmd<void>("open_url", { url });

// ---- forms and images
export interface FormWidget {
  id: [number, number];
  page: number;
  rect: [number, number, number, number];
  on_state: string | null;
}
export interface FormField {
  name: string;
  kind: "text" | "checkbox" | "radio" | "choice" | "button" | "signature";
  value: string;
  options: string[];
  multiline: boolean;
  readonly: boolean;
  widgets: FormWidget[];
}
export const cmdListFields = () => cmd<FormField[]>("list_fields");
export const cmdSetField = (name: string, value: string | boolean) => cmd<PageMeta[]>("set_field", { name, value });
export const cmdInsertImage = (page: number, rect: number[], image: unknown) => cmd<PageMeta[]>("insert_image", { page, rect, image });
export const cmdReadImage = (path: string) => cmd<{ b64: string }>("read_image_file", { path });

// ---- document tabs
export const cmdTabNew = () => cmd<number>("tab_new");
export const cmdTabSwitch = (id: number) => cmd<number>("tab_switch", { id });
export const cmdTabClose = (id: number) => cmd<void>("tab_close", { id });
export const cmdMovePagesToTab = (pages: number[], to: number) => cmd<PageMeta[]>("move_pages_to_tab", { pages, to });

// ---- crash recovery
export interface RecoveryEntry {
  file: string;
  name: string;
  original_path: string | null;
  saved_at: number;
  pid: number;
}
export const cmdSaveRecovery = () => cmd<number>("save_recovery");
export const cmdClearRecovery = () => cmd<void>("clear_recovery");
export const cmdListRecovery = () => cmd<RecoveryEntry[]>("list_recovery");
export const cmdRestoreRecovery = (file: string) => cmd<OpenPayload>("restore_recovery", { file });
export const cmdDiscardRecovery = () => cmd<void>("discard_recovery");

// ---- OCR (Apple Vision helper)
export interface OcrLine {
  text: string;
  x: number;
  y: number;
  w: number;
  h: number;
  confidence: number;
}
export const cmdOcrRecognize = (b64: string, languages?: string) => cmd<OcrLine[]>("ocr_recognize", { b64, languages });
export const cmdOcrApply = (pages: { page: number; rotation: number; lines: OcrLine[] }[]) => cmd<PageMeta[]>("ocr_apply", { pages });
