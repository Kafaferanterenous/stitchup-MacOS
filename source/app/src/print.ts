//! Printing: pages are rasterised (pdf.js, with annotations) into a print-only
//! layer — one sheet per page, fitted inside a 0.25" margin with the same
//! aspect-preserving maths as Fit Page — then the macOS print panel opens.
//! The panel's own preview and "Save as PDF" cover Print Preview on paper and
//! Print to PDF.

import { invoke } from "@tauri-apps/api/core";
import type { RenderSession } from "./adapters/renderAdapter";

export const PAPERS: { name: string; w: number; h: number }[] = [
  { name: "Letter", w: 612, h: 792 },
  { name: "Legal", w: 612, h: 1008 },
  { name: "A4", w: 595, h: 842 },
  { name: "A5", w: 420, h: 595 },
  { name: "A3", w: 842, h: 1191 },
  { name: "Executive", w: 522, h: 756 },
];

export interface PrintOptions {
  pages: number[];
  paper: { w: number; h: number };
  landscape: boolean;
  /** Pixels per point for the raster (2 ≈ 144 dpi, 3 ≈ 216 dpi). */
  quality?: number;
}

export const MARGIN_PT = 18; // 0.25 inch

/** Size of a page drawn on a sheet: fitted, centred, aspect preserved. */
export function fitOnSheet(page: { w: number; h: number }, sheet: { w: number; h: number }) {
  const aw = sheet.w - MARGIN_PT * 2;
  const ah = sheet.h - MARGIN_PT * 2;
  const s = Math.min(aw / page.w, ah / page.h);
  const w = page.w * s;
  const h = page.h * s;
  return { x: (sheet.w - w) / 2, y: (sheet.h - h) / 2, w, h };
}

async function renderPng(session: RenderSession, pageNo: number, rotation: number, scale: number) {
  const page = await session.pdf.getPage(pageNo);
  const vp = page.getViewport({ scale, rotation });
  const c = document.createElement("canvas");
  c.width = Math.ceil(vp.width);
  c.height = Math.ceil(vp.height);
  const ctx = c.getContext("2d")!;
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, c.width, c.height);
  await page.render({ canvasContext: ctx, viewport: vp }).promise;
  return { url: c.toDataURL("image/png"), w: vp.width / scale, h: vp.height / scale };
}

/** Build the print layer and open the print panel. */
export async function printDocument(session: RenderSession, rotations: Map<number, number>, opts: PrintOptions) {
  const sheet = opts.landscape ? { w: opts.paper.h, h: opts.paper.w } : { w: opts.paper.w, h: opts.paper.h };
  document.getElementById("print-root")?.remove();
  document.getElementById("print-style")?.remove();
  const root = document.createElement("div");
  root.id = "print-root";
  for (const n of opts.pages) {
    const img = await renderPng(session, n, rotations.get(n) ?? 0, opts.quality ?? 2.5);
    const f = fitOnSheet({ w: img.w, h: img.h }, sheet);
    const el = document.createElement("div");
    el.className = "print-sheet";
    el.style.width = `${sheet.w}pt`;
    // A hair under the paper height: rounding must never spill onto a second page.
    el.style.height = `${sheet.h - 1}pt`;
    const im = document.createElement("img");
    im.src = img.url;
    im.style.left = `${f.x}pt`;
    im.style.top = `${f.y}pt`;
    im.style.width = `${f.w}pt`;
    im.style.height = `${f.h}pt`;
    el.appendChild(im);
    root.appendChild(el);
  }
  const style = document.createElement("style");
  style.id = "print-style";
  style.textContent = `@page { size: ${sheet.w}pt ${sheet.h}pt; margin: 0; }`;
  document.head.appendChild(style);
  document.body.appendChild(root);
  // Wait for the images to decode so the print panel never sees blanks.
  await Promise.all([...root.querySelectorAll("img")].map((i) => i.decode().catch(() => {})));
  try {
    // Same paper, orientation and zero margins in the macOS print settings.
    await invoke("print_window", { width: opts.paper.w, height: opts.paper.h, landscape: opts.landscape });
  } catch {
    window.print();
  }
  // The layer stays (hidden on screen) until the next print: the native panel
  // is asynchronous and reads it while it is open.
}
