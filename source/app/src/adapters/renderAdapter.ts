//! RendererAdapter over pdfjs-dist (D-03, Apache-2.0).
//! Conformance (STAGE0_ENGINE_INTERFACES §3):
//!  - screen and thumbnail rendering share one code path.
//!  - rotation/zoom are pass-through params; model coordinates stay stable.
//!  - RenderedPage carries pixel size; CSS size is computed by the UI only.

import * as pdfjsLib from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.mjs?url";
import type { Document, TextRun, SearchResult } from "../types";

export interface RenderedPage {
  /** CanvasImageSource-style buffer for display. */
  image: HTMLCanvasElement;
  sizePx: { width: number; height: number };
  sizePt: { width: number; height: number };
  rotation: number;
}

export interface RenderSession {
  doc: Document;
  pdf: pdfjsLib.PDFDocumentProxy;
}

pdfjsLib.GlobalWorkerOptions.workerSrc = workerUrl;

export interface PdfJsLike {
  getDocument: (args: {
    data: ArrayBuffer;
    cMapUrl?: string;
    cMapPacked?: boolean;
    standardFontDataUrl?: string;
  }) => { promise: Promise<pdfjsLib.PDFDocumentProxy> };
}

/** Lazily built pdf.js binding; injected for tests. */
const lib: PdfJsLike = pdfjsLib as unknown as PdfJsLike;

export async function bind(doc: Document): Promise<RenderSession> {
  const bytes = Uint8Array.from(atob(doc.bytesB64), (c) => c.charCodeAt(0));
  const loadingTask = lib.getDocument({
    // Copy so the caller's buffer stays pristine for re-renders.
    data: bytes.slice().buffer as ArrayBuffer,
    // Bundled pdf.js resources (served from the app root): non-embedded
    // standard fonts (Helvetica/Times/Courier...) and CID CMaps. Without
    // these, PDFs that rely on them render with missing/blank text.
    cMapUrl: "cmaps/",
    cMapPacked: true,
    standardFontDataUrl: "standard_fonts/",
  });
  const pdf = await loadingTask.promise;
  return { doc, pdf };
}

export async function unbind(session: RenderSession): Promise<void> {
  try {
    await session.pdf.destroy();
  } catch {
    // already destroyed
  }
}

export async function renderPage(
  session: RenderSession,
  pageNumber: number,
  scale: number,
  rotation: number,
): Promise<RenderedPage> {
  const page = await session.pdf.getPage(pageNumber);
  // Render at devicePixelRatio so the raster fills the whole backing store;
  // pdf.js paints at viewport scale, so a dpr-inflated backing with a CSS-scale
  // viewport would leave the page in the top-left corner on HiDPI screens.
  const dpr = window.devicePixelRatio ?? 1;
  const viewport = page.getViewport({ scale: scale * dpr, rotation: rotation % 360 });
  const canvas = document.createElement("canvas");
  canvas.width = Math.ceil(viewport.width);
  canvas.height = Math.ceil(viewport.height);
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    throw new Error("canvas 2d context unavailable");
  }
  await page.render({
    canvasContext: ctx,
    viewport,
  }).promise;

  const sizePt = { width: viewport.width / (scale * dpr), height: viewport.height / (scale * dpr) };
  return {
    image: canvas,
    sizePx: { width: viewport.width / dpr, height: viewport.height / dpr },
    sizePt,
    rotation,
  };
}

/** Text layer runs for selection + search (never used for in-doc edits). */
export async function getTextRuns(session: RenderSession, pageNumber: number): Promise<TextRun[]> {
  const page = await session.pdf.getPage(pageNumber);
  const content = await page.getTextContent();
  const viewport = page.getViewport({ scale: 1, rotation: 0 });
  const extracted: Array<TextRun | null> = content.items.map((item) => {
    const str = (item as { str?: string }).str;
    const transform = (item as { transform?: number[] }).transform;
    if (typeof str !== "string" || transform === undefined) return null;
    const [a, , , d, e, f]: number[] = transform;
    const width = Math.abs(a);
    const height = Math.abs(d);
    const pt = viewport.convertToViewportPoint(e, f);
    return {
      rect: {
        x: pt[0],
        y: pt[1] - height,
        width,
        height,
      },
      str,
      unicode: str,
      char_rects: [],
    } as TextRun;
  });
  return extracted.filter((x): x is TextRun => x !== null);
}

/** Plain substring search over the text layer (Stage 1; not OCR). */
export async function search(
  session: RenderSession,
  query: string,
  _opts: unknown,
  pageRange: [number, number],
): Promise<SearchResult[]> {
  const q = query.toLocaleLowerCase();
  const hits: SearchResult[] = [];
  for (let p = pageRange[0]; p <= pageRange[1]; p++) {
    const runs = await getTextRuns(session, p);
    for (const run of runs) {
      const idx = run.str.toLocaleLowerCase().indexOf(q);
      if (idx >= 0) {
        hits.push({
          page_no: p,
          rect: { ...run.rect },
          match_text: run.str.slice(idx, idx + q.length),
        });
      }
    }
  }
  return hits;
}