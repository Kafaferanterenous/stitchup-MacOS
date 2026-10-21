//! Render the REAL built frontend (source/app/dist) headlessly and capture a
//! documentation screenshot: ribbon, page thumbnails and the main viewer with a
//! document open. Same Tauri-boundary stub as acceptance.mjs, so the renderer is
//! the shipping pdf.js build and no app source is modified.
//!
//! Run: node tests/ui/screenshot.mjs [fixture.pdf] [out.png]
//! Default: a 3-page demo PDF built with pdf-lib -> docs/screenshot.png

import { chromium } from "playwright";
import http from "node:http";
import { readFile, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { PDFDocument, StandardFonts, rgb } from "pdf-lib";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const APP_DIR = path.resolve(__dirname, "..", ".."); // source/app
const REPO_DIR = path.resolve(APP_DIR, "..", "..");
const DIST = path.join(APP_DIR, "dist");

const OUT = path.resolve(process.argv[3] ?? path.join(REPO_DIR, "docs", "screenshot.png"));

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json",
  ".wasm": "application/wasm",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".ttf": "font/ttf",
  ".otf": "font/otf",
  ".pfb": "application/octet-stream",
  ".bcmap": "application/octet-stream",
  ".map": "application/json",
};

/** A 3-page demo document so the shot shows real pagination, not "Hello". */
async function makeDemoFixture() {
  const doc = await PDFDocument.create();
  const font = await doc.embedFont(StandardFonts.Helvetica);
  const bold = await doc.embedFont(StandardFonts.HelveticaBold);
  const serif = await doc.embedFont(StandardFonts.TimesRoman);
  const sans = await doc.embedFont(StandardFonts.Helvetica);
  const ink = rgb(0.13, 0.15, 0.18);
  const muted = rgb(0.38, 0.42, 0.47);
  const accent = rgb(0.16, 0.44, 0.78);
  const line = (p, y) => p.drawLine({ start: { x: 64, y }, end: { x: 548, y }, thickness: 0.7, color: rgb(0.85, 0.87, 0.9) });

  const wrap = (text, size, width) => {
    const words = text.split(" ");
    const lines = [];
    let cur = "";
    for (const w of words) {
      const next = cur ? `${cur} ${w}` : w;
      if (font.widthOfTextAtSize(next, size) > width && cur) { lines.push(cur); cur = w; }
      else cur = next;
    }
    if (cur) lines.push(cur);
    return lines;
  };

  const page = () => doc.addPage([612, 792]);

  // --- Page 1: cover / what's new -------------------------------------------
  let p = page();
  p.drawRectangle({ x: 0, y: 712, width: 612, height: 80, color: accent });
  p.drawText("Stitchup", { x: 64, y: 742, size: 30, font: bold, color: rgb(1, 1, 1) });
  p.drawText("A fast, native PDF editor for macOS", { x: 64, y: 722, size: 12, font: sans, color: rgb(0.88, 0.93, 1) });
  p.drawText("Release notes — v0.2.3", { x: 64, y: 668, size: 16, font: bold, color: ink });
  line(p, 656);
  let y = 630;
  const bullets = [
    "Real page thumbnails, rendered lazily from a single shared pdf.js session.",
    "Page bookmarks: star a page to keep it close, without touching the file.",
    "Double-click a text line to edit it inline; Escape cancels, Enter commits.",
  ];
  for (const b of bullets) {
    p.drawCircle({ x: 68, y: y + 4, size: 2.2, color: accent });
    for (const l of wrap(b, 11, 462)) { p.drawText(l, { x: 80, y, size: 11, font: serif, color: ink }); y -= 15; }
    y -= 8;
  }
  y -= 6;
  p.drawText("Built for real documents", { x: 64, y, size: 14, font: bold, color: ink });
  y -= 22;
  const para = "Open a PDF, rotate or reorder its pages, extract a range, or merge two documents. Every operation is undoable, and saving is atomic: Stitchup writes to a temporary file first and only then publishes it, so an interrupted save can never corrupt your original.";
  for (const l of wrap(para, 11, 470)) { p.drawText(l, { x: 64, y, size: 11, font: serif, color: ink }); y -= 15; }
  p.drawText("Page 1 of 3", { x: 64, y: 48, size: 9, font: sans, color: muted });
  p.drawText("Stitchup — release notes", { x: 400, y: 48, size: 9, font: sans, color: muted });

  // --- Page 2: page operations ----------------------------------------------
  p = page();
  p.drawText("Page operations", { x: 64, y: 720, size: 20, font: bold, color: ink });
  line(p, 706);
  y = 676;
  const ops = [
    ["Rotate", "90°, 180° or 270°, applied to any selection of pages."],
    ["Reorder", "Drag pages in the sidebar; the document order updates live."],
    ["Delete / Insert", "Remove pages or insert a blank page at a chosen size."],
    ["Extract & merge", "Export a page range as a new PDF, or combine documents."],
    ["Undo / redo", "Every operation is recorded and can be stepped back."],
  ];
  for (const [title, body] of ops) {
    p.drawText(title, { x: 64, y, size: 12, font: bold, color: accent });
    y -= 16;
    for (const l of wrap(body, 11, 460)) { p.drawText(l, { x: 80, y, size: 11, font: serif, color: ink }); y -= 15; }
    y -= 12;
  }
  p.drawText("Page 2 of 3", { x: 64, y: 48, size: 9, font: sans, color: muted });

  // --- Page 3: themes & safety ----------------------------------------------
  p = page();
  p.drawText("Themes & safety", { x: 64, y: 720, size: 20, font: bold, color: ink });
  line(p, 706);
  y = 662;
  const rows = [
    ["Theme", "Classic XP, Dark and Blue, all with readable contrast."],
    ["Search", "Find text in the page layer with hit highlighting."],
    ["Selection", "Select and copy text, including non-Latin scripts."],
    ["Safety", "Source hash check, atomic save, no reflow on edits."],
  ];
  for (const [k, v] of rows) {
    p.drawText(k, { x: 64, y, size: 11, font: bold, color: ink });
    for (const l of wrap(v, 11, 360)) { p.drawText(l, { x: 190, y, size: 11, font: serif, color: ink }); y -= 15; }
    y -= 12;
  }
  p.drawText("Page 3 of 3", { x: 64, y: 48, size: 9, font: sans, color: muted });

  const out = path.join(os.tmpdir(), "stitchup-docs-demo.pdf");
  await writeFile(out, await doc.save());
  return out;
}

function serveDist() {
  const server = http.createServer(async (req, res) => {
    try {
      const urlPath = decodeURIComponent((req.url ?? "/").split("?")[0]);
      let filePath = path.join(DIST, urlPath);
      if (urlPath === "/" || urlPath.endsWith("/")) filePath = path.join(filePath, "index.html");
      if (!existsSync(filePath)) filePath = path.join(DIST, "index.html");
      const body = await readFile(filePath);
      res.setHeader("Content-Type", MIME[path.extname(filePath)] ?? "application/octet-stream");
      res.end(body);
    } catch (e) {
      res.statusCode = 500;
      res.end(String(e));
    }
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => resolve({ server, port: server.address().port }));
  });
}

async function buildOpenResult(fixturePath) {
  const bytes = await readFile(fixturePath);
  const pdf = await PDFDocument.load(bytes, { ignoreEncryption: true });
  const pages = pdf.getPages().map((page, i) => {
    const { width, height } = page.getSize();
    const angle = ((page.getRotation().angle % 360) + 360) % 360;
    return {
      page_no: i + 1,
      width_pt: Math.round(width),
      height_pt: Math.round(height),
      rotation: angle,
      has_annotations: false,
    };
  });
  return {
    path: fixturePath,
    source_hash: createHash("sha256").update(bytes).digest("hex"),
    source_size: bytes.length,
    page_count: pages.length,
    pages,
    metadata: { title: "Stitchup — release notes" },
    encrypted: false,
    password_required: false,
    warnings: [],
    bytes_b64: bytes.toString("base64"),
  };
}

const INK = `(canvas) => {
  const ctx = canvas.getContext('2d');
  if (!ctx || !canvas.width || !canvas.height) return 0;
  const d = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
  let n = 0;
  for (let i = 0; i < d.length; i += 4) {
    if (d[i + 3] > 128 && d[i] < 200 && d[i + 1] < 200 && d[i + 2] < 200) n++;
  }
  return n;
}`;

async function main() {
  if (!existsSync(DIST)) throw new Error(`dist/ not found — run \`npm run build\` first (${DIST})`);

  const FIXTURE = process.argv[2] ? path.resolve(process.argv[2]) : await makeDemoFixture();
  if (!existsSync(FIXTURE)) throw new Error(`fixture not found: ${FIXTURE}`);

  const openResult = await buildOpenResult(FIXTURE);
  const savePath = "/tmp/stitchup-screenshot-out.pdf";
  const { server, port } = await serveDist();
  const base = `http://127.0.0.1:${port}/`;

  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: { width: 1400, height: 900 },
    deviceScaleFactor: 2,
  });
  await context.addInitScript(
    ({ openResult, fixturePath, savePath }) => {
      window.__TAURI_INTERNALS__ = {
        transformCallback: (cb) => {
          const id = Math.floor(Math.random() * 1e9);
          window[`_${id}`] = cb;
          return id;
        },
        unregisterCallback: () => {},
        convertFileSrc: (p) => p,
        invoke: async (cmd) => {
          switch (cmd) {
            case "open_document":
            case "unlock_document":
              return openResult;
            case "plugin:dialog|open":
              return fixturePath;
            case "plugin:dialog|save":
              return savePath;
            case "render_bytes":
              return { bytes_b64: openResult.bytes_b64 };
            case "page_op":
            case "cmd_set_metadata":
            case "undo":
            case "redo":
            case "merge_document":
              return openResult.pages;
            case "source_hash":
              return { source_hash: openResult.source_hash };
            case "commit_export":
              return {
                new_sha256: openResult.source_hash,
                size_bytes: openResult.source_size,
                published_path: savePath,
              };
            case "quit_app":
              return null;
            default:
              throw new Error(`unmocked Tauri command: ${cmd}`);
          }
        },
      };
      window.isTauri = true;
    },
    { openResult, fixturePath: FIXTURE, savePath },
  );

  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  try {
    await page.goto(base, { waitUntil: "domcontentloaded" });
    await page.waitForSelector(".app-shell", { timeout: 15000 });
    await page.locator('[title="Open a PDF (⌘O)"]').first().click();

    await page.waitForFunction(
      ({ n, inkFn }) => {
        const f = eval(inkFn);
        const nodes = [...document.querySelectorAll("canvas.thumb-canvas")];
        return nodes.length === n && nodes.every((c) => f(c) > 0);
      },
      { n: openResult.page_count, inkFn: INK },
      { timeout: 30000 },
    );

    // Fit the whole page in the viewport (zoom shortcuts; fit modes only set
    // zoomMode today, so drive the zoom itself).
    await page.keyboard.press("Meta+-");
    await page.keyboard.press("Meta+-");

    await page.waitForFunction(
      (inkFn) => {
        const f = eval(inkFn);
        const c = document.querySelector(".page-view canvas");
        const h = c?.getBoundingClientRect().height ?? 0;
        return !!c && f(c) > 0 && h > 0 && h < 750;
      },
      INK,
      { timeout: 30000 },
    );

    // Star page 2 so the bookmark affordance is visible (session-only).
    if (openResult.page_count > 1) {
      await page.locator(".thumb").nth(1).locator(".thumb-bookmark").click().catch(() => {});
    }

    const state = await page.evaluate(() => {
      const v = document.querySelector(".page-view canvas");
      const r = v?.getBoundingClientRect();
      let bands = null;
      if (v && v.width) {
        const d = v.getContext("2d").getImageData(0, 0, v.width, v.height).data;
        const rows = [];
        for (let y = 0; y < v.height; y++) {
          let n = 0;
          for (let x = 0; x < v.width; x++) {
            const o = (y * v.width + x) * 4;
            if (d[o] < 180 && d[o + 1] < 180 && d[o + 2] < 180 && d[o + 3] > 128) n++;
          }
          rows.push(n);
        }
        bands = [];
        let s0 = -1;
        rows.forEach((n, y) => {
          if (n > 10) { if (s0 < 0) s0 = y; } else if (s0 >= 0) { bands.push([s0, y - 1]); s0 = -1; }
        });
        if (s0 >= 0) bands.push([s0, v.height - 1]);
      }
      return {
        thumbs: document.querySelectorAll("canvas.thumb-canvas").length,
        viewer: r ? [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)] : null,
        backing: v ? [v.width, v.height] : null,
        bands,
      };
    });
    console.log("state:", JSON.stringify(state));

    await page.mouse.move(1390, 890);
    await page.waitForTimeout(700);
    await page.screenshot({ path: OUT, fullPage: false });

    const realErrors = errors.filter((e) => !/favicon/i.test(e));
    if (realErrors.length) console.log(`console errors: ${realErrors.slice(0, 3).join(" | ")}`);
  } finally {
    await context.close();
    await browser.close();
    server.close();
  }
  console.log(`screenshot: ${OUT}`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
