//! Model + engine interface types shared across the frontend.
//! These mirror docs/STAGE0_DOCUMENT_MODEL.md and the Rust `stitchup-core`
//! serde payloads (field-for-field, snake_case). The UI imports engine types
//! ONLY through the adapters — never raw pdf.js/pdf-lib types.

/** Error codes: STAGE0_ENGINE_INTERFACES §1 (must equal the Rust enum). */
export type ErrorCode =
  | "open_failed"
  | "encrypted"
  | "password_required"
  | "malformed"
  | "unsupported"
  | "permission_denied"
  | "io_error"
  | "cancelled"
  | "atomic_publish_failed"
  | "validation_failed";

export interface EngineError {
  code: ErrorCode;
  message: string;
  detail?: unknown;
}

export function isEngineError(e: unknown): e is EngineError {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as EngineError).message === "string" &&
    typeof (e as EngineError).code === "string"
  );
}

/** PageMeta — structural page facts (from the Rust core). */
export interface PageMeta {
  page_no: number;
  width_pt: number;
  height_pt: number;
  rotation: number;
  has_annotations: boolean;
}

/** Metadata — Info dictionary projection. */
export interface Metadata {
  title?: string;
  author?: string;
  subject?: string;
  keywords?: string;
  creator?: string;
  producer?: string;
}

/** OpenResult — the adapter contract type. */
export interface OpenResult {
  path: string;
  source_hash: string;
  source_size: number;
  page_count: number;
  pages: PageMeta[];
  metadata: Metadata;
  encrypted: boolean;
  password_required: boolean;
  warnings: string[];
  /** Base64 of the opened bytes; the RendererAdapter consumes this. */
  bytes_b64: string;
}

/** CommitResult — atomic save-as outcome. */
export interface CommitResult {
  new_sha256: string;
  size_bytes: number;
  published_path: string;
}

/** Operation — returned by every PageTree/Metadata mutation (for undo/redo). */
export interface Operation {
  kind:
    | "reorder"
    | "rotate"
    | "delete"
    | "insert_blank"
    | "extract"
    | "edit_text"
    | "set_metadata"
    | "merge"
    | "open";
  /** Page numbers BEFORE the op (1-based) — revert reference. */
  before: number[];
  /** For set_metadata: snapshot of the prior Metadata (exact inverse). */
  beforeMeta?: Metadata;
  /** Mutation payload, adapter-specific. */
  data: unknown;
}

/** Text edit op: white patch + replacement text over `rect` (PDF user space, bottom-left origin, points). */
export interface TextEditParams {
  page: number;
  x: number;
  y: number;
  width: number;
  height: number;
  font_size: number;
  new_text: string;
}

export type ZoomMode = "fit-page" | "fit-width" | "actual" | "custom";
export type ViewMode = "single" | "spread" | "continuous";

export interface SearchResult {
  page_no: number;
  rect: { x: number; y: number; width: number; height: number };
  match_text: string;
}

export interface TextRun {
  rect: { x: number; y: number; width: number; height: number };
  str: string;
  unicode: string;
  char_rects: { x: number; y: number; width: number; height: number }[];
}

/** In-memory model (STAGE0_DOCUMENT_MODEL §2). */
export interface Document {
  /** SHA-256 of the originally opened bytes. NEVER replaced by a save. */
  sourceHash: string;
  sourceSize: number;
  path: string;
  metadata: Metadata;
  pages: PageState[];
  encrypted: boolean;
  passwordRequired: boolean;
  warnings: string[];
  /** Base64 opened bytes; the RendererAdapter's input. */
  bytesB64: string;
  /** Client-side page bookmarks (not persisted into the PDF). */
  bookmarks: number[];
  /** Live operation stack for the CommandBus (undo/redo). */
  history: Operation[];
  redo: Operation[];
}

export interface PageState extends PageMeta {
  /** Renderer-produced pixel buffer for the "current" scale (adapter-owned). */
  thumb?: Uint8Array;
}

// ---------------------------------------------------------------------------
// Adapter interfaces (language shape per STAGE0_ENGINE_INTERFACES)
// ---------------------------------------------------------------------------

export interface OpenDocumentAdapter {
  open(path: string): Promise<Document>;
  verifyIntegrity(doc: Document): Promise<{ sha256: string; size: number; pages: number; warnings: string[] }>;
}

export interface PageTreeAdapter {
  reorder(doc: Document, newOrder: number[]): Promise<Operation>;
  rotate(doc: Document, pageNumbers: number[], cw: boolean): Promise<Operation>;
  delete(doc: Document, pageNumbers: number[]): Promise<Operation>;
  insertBlank(doc: Document, afterPage: number, widthPt: number, heightPt: number): Promise<Operation>;
  extract(doc: Document, pageNumbers: number[]): Promise<{ pages: PageState[]; outSourceHash: string }>;
  editText(doc: Document, params: TextEditParams): Promise<Operation>;
  merge(doc: Document, path: string): Promise<Operation>;
  getPage(doc: Document, pageNumber: number): PageState | undefined;
}

export interface MetadataAdapter {
  get(doc: Document): Metadata;
  set(doc: Document, updates: Metadata): Promise<Operation>;
}

export interface CommitAdapter {
  export(doc: Document, target: string, overwrite: boolean): Promise<CommitResult>;
  /** Permission/invariant preflight; refuses target == source path. */
  validate(doc: Document, target: string): Promise<{ ok: boolean; reason?: string }>;
}