//! Headless UI acceptance for Stage 1.8 (real page thumbnails, page bookmarks,
//! double-click inline text edit).
//!
//! The app is a Tauri webview: the frontend talks to the Rust core only through
//! `@tauri-apps/api`'s `invoke`, which dispatches to
//! `window.__TAURI_INTERNALS__.invoke(cmd, args)`. This harness loads the REAL
//! built frontend (source/app/dist) in headless Chromium, serves it from a tiny
//! static server, and stubs `__TAURI_INTERNALS__.invoke` with an OpenResult that
//! is derived from a real fixture PDF. Nothing in the app is modified — the
//! renderer (pdf.js) is the real one, so what renders here is what ships.
//!
//! Run: node tests/ui/acceptance.mjs [fixture.pdf]
//! Exit code 0 = all assertions passed.

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
const DIST = path.join(APP_DIR, "dist");

// The app's own generated fixtures (fixtures/generated/F-*.pdf) are written by
// the Rust core and declare FlateDecode over raw content streams that pdf.js
// refuses to inflate ("Unknown compression method in flate stream"), so they
// render blank in the webview. For a UI acceptance test we need a PDF pdf.js
// can actually render, so we synthesize a valid one with pdf-lib (a real app
// dependency) unless the caller passes a known-good file path.
async function makeFixture() {
  const doc = await PDFDocument.create();
  const font = await doc.embedFont(StandardFonts.Helvetica);
  for (let i = 1; i <= 3; i++) {
    const p = doc.addPage([612, 792]);
    p.drawText(`Stitchup acceptance page ${i}`, { x: 72, y: 700, size: 24, font, color: rgb(0, 0, 0) });
    p.drawText("The quick brown fox jumps over the lazy dog.", { x: 72, y: 650, size: 14, font });
  }
  const p = path.join(os.tmpdir(), "stitchup-ui-acceptance-fixture.pdf");
  await writeFile(p, await doc.save());
  return p;
}

const FIXTURE = process.argv[2] ? path.resolve(process.argv[2]) : await makeFixture();

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
  ".pdf": "application/pdf",
  ".map": "application/json",
};

function serveDist() {
  const server = http.createServer(async (req, res) => {
    try {
      const urlPath = decodeURIComponent((req.url ?? "/").split("?")[0]);
      let filePath = path.join(DIST, urlPath);
      if (urlPath === "/" || urlPath.endsWith("/")) filePath = path.join(filePath, "index.html");
      if (!existsSync(filePath)) filePath = path.join(DIST, "index.html"); // SPA fallback
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

/** Build an authentic OpenResult for a fixture from its bytes + pdf-lib. */
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
    metadata: { title: "UI acceptance fixture" },
    encrypted: false,
    password_required: false,
    warnings: [],
    bytes_b64: bytes.toString("base64"),
  };
}

const results = [];
function check(name, ok, detail = "") {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? ` — ${detail}` : ""}`);
}

// Count "ink" pixels: opaque AND non-white. A never-drawn canvas is transparent
// (alpha 0) so it never counts; a rendered page is white with dark glyphs.
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
  if (!existsSync(FIXTURE)) throw new Error(`fixture not found: ${FIXTURE}`);

  const openResult = await buildOpenResult(FIXTURE);
  const savePath = "/tmp/stitchup-ui-acceptance-out.pdf";
  const { server, port } = await serveDist();
  const base = `http://127.0.0.1:${port}/`;
  console.log(`serving ${DIST} at ${base}`);
  console.log(`fixture: ${FIXTURE} (${openResult.page_count} pages)`);

  const browser = await chromium.launch();
  const context = await browser.newContext({ viewport: { width: 1400, height: 900 } });
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
  const consoleErrors = [];
  page.on("pageerror", (e) => consoleErrors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") consoleErrors.push(m.text());
  });

  try {
    await page.goto(base, { waitUntil: "domcontentloaded" });
    await page.waitForSelector(".app-shell", { timeout: 15000 });

    // --- Open the document via the ribbon Quick Access "Open" (real dialog path)
    await page.locator('[title="Open a PDF (⌘O)"]').first().click();

    // --- Thumbnails: one canvas per page, all rasterised to real ink ---------
    await page.waitForFunction(
      ({ n, inkFn }) => {
        const f = eval(inkFn);
        const nodes = [...document.querySelectorAll("canvas.thumb-canvas")];
        return nodes.length === n && nodes.every((c) => f(c) > 0);
      },
      { n: openResult.page_count, inkFn: INK },
      { timeout: 30000 },
    );
    const thumbInk = await page.$$eval("canvas.thumb-canvas", (nodes, inkFn) => {
      const f = eval(inkFn);
      return nodes.map((c) => f(c));
    }, INK);
    check(
      "sidebar thumbnails render real ink",
      thumbInk.length === openResult.page_count && thumbInk.every((n) => n > 0),
      thumbInk.join(","),
    );

    // --- Main viewer canvas has ink (page paints after React commits size) ---
    const viewerInk = await page.waitForFunction(
      (inkFn) => {
        const f = eval(inkFn);
        const c = document.querySelector(".page-view canvas");
        return !!c && f(c) > 0;
      },
      INK,
      { timeout: 30000 },
    );
    check("main viewer paints page ink", !!viewerInk, "");

    // --- Text layer present (selection/edit surface) -------------------------
    const textRuns = await page.locator(".text-run").count();
    check("text layer has selectable runs", textRuns > 0, `${textRuns} runs`);

    // --- Bookmarks: star toggle flips state + Bookmarks tab count ------------
    const firstThumb = page.locator(".thumb").first();
    await firstThumb.locator(".thumb-bookmark").click();
    await page.waitForFunction(
      () => document.querySelector(".thumb.bookmarked .thumb-bookmark.on") !== null,
      undefined,
      { timeout: 5000 },
    );
    const star = (await firstThumb.locator(".thumb-bookmark").textContent())?.trim();
    const bmTab = (await page.locator('.pane-tab:has-text("Bookmarks")').textContent())?.trim();
    check("star toggle bookmarks the page", star === "★", `star=${star}`);
    check("Bookmarks tab reflects the count", /\(1\)/.test(bmTab ?? ""), bmTab);

    // Bookmarks tab lists the entry with a mini preview
    await page.locator('.pane-tab:has-text("Bookmarks")').click();
    await page.waitForSelector(".bm-row", { timeout: 5000 });
    const bmInk = await page.$$eval(".bm-row canvas.thumb-canvas", (nodes, inkFn) => {
      const f = eval(inkFn);
      return nodes.map((c) => f(c));
    }, INK);
    check("bookmark entry lists", (await page.locator(".bm-row").count()) === 1, "");
    check("bookmark mini-preview renders", bmInk.some((n) => n > 0), bmInk.join(","));

    // back to Pages
    await page.locator('.pane-tab:has-text("Pages")').click();

    // --- Inline text edit: double-click a run opens the inline editor --------
    await page.locator(".text-run").first().dblclick();
    await page.waitForSelector(".inline-editor", { timeout: 5000 });
    const editorValue = await page.locator(".inline-editor").inputValue();
    check("double-click opens inline editor", editorValue.length >= 0, `value="${editorValue}"`);
    // Escape cancels without committing
    await page.locator(".inline-editor").press("Escape");
    await page.waitForFunction(() => document.querySelector(".inline-editor") === null, undefined, {
      timeout: 5000,
    });
    check("Escape closes the inline editor", true, "");
  } finally {
    await page.screenshot({ path: "/tmp/stitchup-ui-acceptance.png", fullPage: false }).catch(() => {});
    await context.close();
    await browser.close();
    server.close();
  }

  const realErrors = consoleErrors.filter((e) => !/favicon/i.test(e));
  check("no uncaught page/console errors", realErrors.length === 0, realErrors.slice(0, 3).join(" | "));

  const failed = results.filter((r) => !r.ok);
  console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
  if (failed.length) {
    console.log(`screenshot: /tmp/stitchup-ui-acceptance.png`);
    process.exit(1);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
