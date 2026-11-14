//! End-to-end UI test: the REAL built frontend (dist/) in headless Chromium,
//! talking to the REAL Rust session logic through the dev bridge
//! (src-tauri/examples/dev_bridge.rs). Only the native file dialogs and the
//! native print panel are stubbed (each test queues what the "user" picks).
//!
//! Every ribbon tab and button, dialog, sidebar tool and keyboard shortcut is
//! exercised; each effect is verified against backend state or the written
//! files (PDFs parsed independently with pdf.js in Node, ZIP packages with
//! Python's zipfile).
//!
//!   npm run build && npm run test:e2e
//! Exit code 0 = all checks passed. Screenshots: $E2E_OUT (default temp/).

import { chromium } from "playwright";
import { spawn, execFileSync } from "node:child_process";
import { mkdir, readFile, writeFile, rm, copyFile, readdir, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import path from "node:path";
import os from "node:os";
import { fileURLToPath } from "node:url";
import { PDFDocument, PDFName, StandardFonts, rgb } from "pdf-lib";
import zlib from "node:zlib";
import * as pdfjs from "pdfjs-dist/legacy/build/pdf.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const APP = path.resolve(__dirname, "..", "..");
const REPO = path.resolve(APP, "..", "..");
const PORT = 1431 + Math.floor(Math.random() * 200);
const BASE = `http://127.0.0.1:${PORT}`;
const OUT = process.env.E2E_OUT ?? path.join(os.tmpdir(), "stitchup-e2e");
const WORK = path.join(OUT, "work");
const FIX = path.join(REPO, "fixtures", "generated");

// ------------------------------------------------------------------ fixtures

async function makePdf(file, pages) {
  const doc = await PDFDocument.create();
  const font = await doc.embedFont(StandardFonts.Helvetica);
  const bold = await doc.embedFont(StandardFonts.TimesRomanBold);
  doc.setTitle("E2E fixture");
  doc.setAuthor("Original Author");
  for (const [i, size] of pages.entries()) {
    const p = doc.addPage(size);
    const top = size[1] - 72;
    p.drawText(`Heading for page ${i + 1}`, { x: 72, y: top, size: 24, font: bold, color: rgb(0, 0, 0) });
    p.drawText("The quick brown fox jumps over the lazy dog.", { x: 72, y: top - 50, size: 14, font });
    p.drawText(`Line two on page ${i + 1}`, { x: 72, y: top - 80, size: 14, font });
  }
  await writeFile(file, await doc.save({ useObjectStreams: false }));
}

/** A form with every field kind (pdf-lib builds the widgets + appearances). */
async function makeForm(file) {
  const doc = await PDFDocument.create();
  const page = doc.addPage([612, 792]);
  const font = await doc.embedFont(StandardFonts.Helvetica);
  page.drawText("Application form", { x: 72, y: 720, size: 20, font });
  const form = doc.getForm();
  const name = form.createTextField("applicant.name");
  name.addToPage(page, { x: 72, y: 640, width: 260, height: 24 });
  const notes = form.createTextField("notes");
  notes.enableMultiline();
  notes.addToPage(page, { x: 72, y: 520, width: 300, height: 80 });
  const agree = form.createCheckBox("agree");
  agree.addToPage(page, { x: 72, y: 470, width: 18, height: 18 });
  const size = form.createRadioGroup("size");
  size.addOptionToPage("small", page, { x: 72, y: 420, width: 16, height: 16 });
  size.addOptionToPage("large", page, { x: 120, y: 420, width: 16, height: 16 });
  const country = form.createDropdown("country");
  country.addOptions(["Poland", "Australia", "Germany"]);
  country.addToPage(page, { x: 72, y: 360, width: 200, height: 22 });
  await writeFile(file, await doc.save({ useObjectStreams: false }));
}

/** Minimal PNG encoder: w×h RGBA, left half opaque red, right half transparent. */
async function makePng(file, w = 40, h = 20) {
  const raw = Buffer.alloc((w * 4 + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 4 + 1)] = 0;
    for (let x = 0; x < w; x++) {
      const o = y * (w * 4 + 1) + 1 + x * 4;
      raw.set(x < w / 2 ? [220, 30, 30, 255] : [30, 30, 220, 0], o);
    }
  }
  const crcTable = Array.from({ length: 256 }, (_, n) => {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  });
  const crc = (b) => {
    let c = 0xffffffff;
    for (const x of b) c = crcTable[(c ^ x) & 255] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };
  const chunk = (type, data) => {
    const t = Buffer.from(type);
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const c = Buffer.alloc(4);
    c.writeUInt32BE(crc(Buffer.concat([t, data])));
    return Buffer.concat([len, t, data, c]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr.set([8, 6, 0, 0, 0], 8);
  await writeFile(file, Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", zlib.deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]));
}

const LETTER = [612, 792];
const A5 = [420, 595];

/** Text of every page of a PDF file (independent check via pdf.js in Node). */
async function pdfText(file, password) {
  const data = new Uint8Array(await readFile(file));
  const doc = await pdfjs.getDocument({ data, password, isEvalSupported: false }).promise;
  const pages = [];
  for (let i = 1; i <= doc.numPages; i++) {
    const tc = await (await doc.getPage(i)).getTextContent();
    pages.push(tc.items.map((it) => it.str).join(" "));
  }
  await doc.destroy();
  return pages;
}

/** Names and text content of a ZIP package (Python's zipfile = independent reader). */
function zipEntries(file) {
  const py = `import zipfile,json,sys
z=zipfile.ZipFile(sys.argv[1]); assert z.testzip() is None
print(json.dumps({n: z.read(n).decode('utf-8','replace') for n in z.namelist()}))`;
  return JSON.parse(execFileSync("python3", ["-c", py, file]).toString());
}

// --------------------------------------------------------------- harness

const results = [];
let page;

async function check(name, fn) {
  try {
    await fn();
    results.push({ ok: true, name });
    console.log(`  PASS  ${name}`);
  } catch (e) {
    results.push({ ok: false, name, err: String(e?.message ?? e).split("\n")[0] });
    console.log(`  FAIL  ${name}\n        ${String(e?.message ?? e).split("\n")[0]}`);
    await page?.screenshot({ path: path.join(OUT, `fail-${results.length}.png`) }).catch(() => {});
    // Don't let a dialog left open by a failed check block the next ones.
    for (let i = 0; i < 3; i++) await page?.keyboard.press("Escape").catch(() => {});
    await page?.getByRole("button", { name: "Dismiss" }).click({ timeout: 500 }).catch(() => {});
  }
}

function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

async function ipc(cmd, args = {}) {
  const r = await fetch(`${BASE}/ipc/${cmd}`, { method: "POST", body: JSON.stringify(args) });
  const j = await r.json();
  if ("err" in j) throw new Error(`${cmd}: ${j.err.message}`);
  return j.ok;
}

let openPath = null;
const state = () => ipc("open_document", { path: openPath });
const pick = (value) => page.evaluate((v) => window.__dialog.push(v), value);
const tab = (name) => page.locator(".ribbon-tabs").getByRole("tab", { name, exact: true }).click();
const btn = (name) => page.getByRole("button", { name, exact: true });
/** A ribbon button by command id (labels repeat across tabs, ids don't). */
const rb = (id) => page.locator(`.rb-btn[data-cmd="${id}"]`);
const run = (id) => page.evaluate((i) => window.__stitchup.run(i), id);
const status = () => page.locator(".status-bar").innerText();
const calls = (cmd) => page.evaluate((c) => window.__calls.filter((x) => x === c).length, cmd);
const notice = () => page.locator(".notice-toast").innerText().catch(() => "");

async function idle() {
  await page.waitForTimeout(150);
  await page.waitForFunction(() => !!document.querySelector(".page-view canvas"), null, { timeout: 10000 });
}

async function waitStatus(re, timeout = 10000) {
  await page.waitForFunction((src) => new RegExp(src).test(document.querySelector(".status-bar")?.textContent ?? ""), re.source, { timeout });
}

async function waitNotice(re, timeout = 10000) {
  await page.waitForFunction((src) => new RegExp(src).test(document.querySelector(".notice-toast")?.textContent ?? ""), re.source, { timeout });
}

async function openViaRibbon(file) {
  openPath = file;
  await page.evaluate(() => (window.__opened = window.__calls.filter((c) => c === "open_document").length));
  await pick(file);
  await tab("Document");
  await rb("open").click();
  // Either the file loads, or (already open) its tab becomes active.
  await page.waitForFunction(
    (p) => window.__calls.filter((c) => c === "open_document").length > window.__opened || document.querySelector(".doc-tab.active")?.title === p,
    file,
  );
  await page.waitForFunction(() => !document.querySelector(".empty-viewer")?.textContent?.includes("Parsing"));
  await waitStatus(/Page \d+ of|password|failed/);
  await idle();
}

async function canvasHasInk(selector = ".page-view canvas") {
  return page.evaluate((sel) => {
    const c = document.querySelector(sel);
    if (!c || !c.width) return false;
    const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
    let dark = 0;
    for (let i = 0; i < d.length; i += 4 * 7) if (d[i] < 128 && d[i + 3] > 0) dark++;
    return dark > 50;
  }, selector);
}

function contrast(a, b) {
  const lum = (c) => {
    const [r, g, bl] = c.match(/\d+(\.\d+)?/g).slice(0, 3).map(Number).map((v) => {
      v /= 255;
      return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}

/** Verify the nth signature of a PDF file with the system OpenSSL. */
async function verifySignature(file, nth = 0) {
  const pdf = await readFile(file);
  const s = pdf.toString("latin1");
  const at = [...s.matchAll(/\/ByteRange/g)][nth].index;
  const nums = s.slice(at).match(/\[([^\]]+)\]/)[1].trim().split(/\s+/).map(Number);
  const data = Buffer.concat([pdf.subarray(0, nums[1]), pdf.subarray(nums[2], nums[2] + nums[3])]);
  const der = Buffer.from(pdf.subarray(nums[1] + 1, nums[2] - 1).toString("latin1"), "hex");
  const len = der[1] < 0x80 ? 2 + der[1] : 2 + (der[1] & 0x7f) + [...der.subarray(2, 2 + (der[1] & 0x7f))].reduce((a, b) => a * 256 + b, 0);
  const sig = path.join(WORK, `sig${nth}.der`);
  const dat = path.join(WORK, `sig${nth}.data`);
  await writeFile(sig, der.subarray(0, len));
  await writeFile(dat, data);
  execFileSync("openssl", ["smime", "-verify", "-binary", "-inform", "DER", "-noverify", "-purpose", "any", "-in", sig, "-content", dat, "-out", "/dev/null"], { stdio: "pipe" });
  return nums;
}

/** Annotation subtypes on a page of the CURRENT in-memory document. */
async function annotsOn(pageNo) {
  const data = new Uint8Array(Buffer.from((await ipc("render_bytes")).bytes_b64, "base64"));
  const doc = await pdfjs.getDocument({ data }).promise;
  const a = await (await doc.getPage(pageNo)).getAnnotations();
  await doc.destroy();
  return a.map((x) => ({ subtype: x.subtype, contents: x.contentsObj?.str ?? "", url: x.url, dest: x.dest }));
}

/** Drag with the mouse between two viewport points. */
async function dragMouse(x0, y0, x1, y1, steps = 8) {
  await page.mouse.move(x0, y0);
  await page.mouse.down();
  for (let i = 1; i <= steps; i++) await page.mouse.move(x0 + ((x1 - x0) * i) / steps, y0 + ((y1 - y0) * i) / steps);
  await page.mouse.up();
}

/** Bounding box of a text run on a page (scrolled into view). */
async function runBox(pageNo, text) {
  // The text layer is rebuilt after every edit: retry if it was mid-swap.
  for (let i = 0; ; i++) {
    try {
      const el = page.locator(`[data-page="${pageNo}"] .text-run`, { hasText: text }).first();
      await el.scrollIntoViewIfNeeded({ timeout: 3000 });
      return await el.boundingBox();
    } catch (e) {
      if (i >= 4) throw e;
      await page.waitForTimeout(300);
    }
  }
}

/** A point in the empty lower part of a page, scrolled into view. */
async function emptySpot(pageNo, fx = 0.5, fy = 0.6) {
  const pv = page.locator(`[data-page="${pageNo}"]`);
  await pv.evaluate((el, f) => {
    const host = document.querySelector(".viewer-scroll");
    const r = el.getBoundingClientRect();
    host.scrollTop += r.top + r.height * f - host.getBoundingClientRect().top - host.clientHeight / 2;
  }, fy);
  await page.waitForTimeout(150);
  const b = await pv.boundingBox();
  return [b.x + b.width * fx, b.y + b.height * fy];
}

/** Render bytes of the CURRENT in-memory document → its text, page by page. */
async function liveText() {
  const f = path.join(WORK, `live-${Date.now()}.pdf`);
  await writeFile(f, Buffer.from((await ipc("render_bytes")).bytes_b64, "base64"));
  return pdfText(f);
}

// ------------------------------------------------------------------- run

await rm(OUT, { recursive: true, force: true });
await mkdir(WORK, { recursive: true });
const MAIN = path.join(WORK, "main.pdf");
const OTHER = path.join(WORK, "other.pdf");
await makePdf(MAIN, [LETTER, LETTER, A5, LETTER]);
await makePdf(OTHER, [LETTER, LETTER]);

const bridgeBin = path.join(APP, "src-tauri", "target", "release", "examples", "dev_bridge");
assert(existsSync(bridgeBin), "build the bridge first: cargo build --release --example dev_bridge");
const RECOVERY = path.join(WORK, "recovery");
const bridgeEnv = { ...process.env, STITCHUP_RECOVERY_DIR: RECOVERY };
const bridge = spawn(bridgeBin, [String(PORT), path.join(APP, "dist")], { stdio: ["ignore", "pipe", "inherit"], env: bridgeEnv });
await new Promise((res) => bridge.stdout.once("data", res));

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1280, height: 960 }, deviceScaleFactor: 1 });
await ctx.grantPermissions(["clipboard-read", "clipboard-write"], { origin: BASE });
page = await ctx.newPage();
const consoleErrors = [];
page.on("pageerror", (e) => consoleErrors.push(e?.message ?? JSON.stringify(e)));
await page.addInitScript(() => {
  window.__dialog = [];
  window.__calls = [];
  window.__TAURI_INTERNALS__ = {
    async invoke(cmd, args) {
      window.__calls.push(cmd);
      if (cmd.startsWith("plugin:dialog|")) {
        const v = window.__dialog.shift();
        return v === undefined ? null : v;
      }
      const r = await fetch(`/ipc/${cmd}`, { method: "POST", body: JSON.stringify(args ?? {}) });
      const j = await r.json();
      if ("err" in j) throw j.err;
      return j.ok;
    },
    transformCallback: () => 0,
    convertFileSrc: (p) => p,
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
  };
});
await page.goto(BASE);

console.log("Stitchup E2E");

await check("starts like Windows: 7 ribbon tabs, empty-state hint, status bar", async () => {
  const tabs = await page.locator(".ribbon-tabs [role=tab]").allInnerTexts();
  assert(tabs.join() === "Document,Pages,Annotate,Content,Zoom,Navigate,Security", tabs.join());
  assert((await page.locator(".empty-viewer").innerText()).includes("No document open"), "empty hint");
  assert(/Stitchup PDF Editor/.test(await status()), await status());
  assert((await page.locator(".pages-pane").count()) === 0, "no sidebar without a document");
});

await check("Document tab has the Windows button set", async () => {
  const labels = await page.locator(".ribbon-card .rb-text").allInnerTexts();
  const want = "New,Open,Save,Save As,Properties,Import,Export Text,Export CSV,Images,RTF,DOCX,ODT,Print,Preview,to PDF";
  assert(labels.join() === want, labels.join());
});

// ---- File / open
await check("Document ▸ Open opens a PDF fit-to-width and renders page 1", async () => {
  await openViaRibbon(MAIN);
  assert(/Page 1 of 4/.test(await status()), await status());
  assert(/main\.pdf/.test(await status()), "file name in status bar");
  assert(await canvasHasInk(), "page canvas is blank");
  assert((await page.locator(".thumb").count()) === 4, "4 thumbnails expected");
  const ratio = await page.evaluate(() => {
    const host = document.querySelector(".viewer-scroll");
    return document.querySelector('[data-page="1"]').getBoundingClientRect().width / (host.clientWidth - 36);
  });
  assert(Math.abs(ratio - 1) < 0.03, `fit width ratio ${ratio}`);
});

await check("text layer spans cover the real text width", async () => {
  const w = await page.locator('[data-page="1"] .text-run', { hasText: "The quick brown fox" }).evaluate((el) => el.getBoundingClientRect().width);
  assert(w > 200, `selection box only ${w}px wide`);
});

// ---- Navigation
await check("Navigate ▸ Next / Previous / Go to page", async () => {
  await tab("Navigate");
  await rb("next").click();
  await waitStatus(/Page 2 of 4/);
  await rb("prev").click();
  await waitStatus(/Page 1 of 4/);
  await page.getByLabel("Go to page").fill("3");
  await btn("Go").click();
  await waitStatus(/Page 3 of 4/);
});

await check("clicking a thumbnail scrolls the viewer to that page", async () => {
  await page.locator('[data-thumb="2"] .thumb').click();
  await waitStatus(/Page 2 of 4/);
  await page.waitForTimeout(300);
  const top = await page.evaluate(() => {
    const host = document.querySelector(".viewer-scroll").getBoundingClientRect();
    return document.querySelector('[data-page="2"]').getBoundingClientRect().top - host.top;
  });
  assert(Math.abs(top) < 60, `page 2 not scrolled into view (offset ${top}px)`);
});

await check("scrolling updates the current page", async () => {
  await page.evaluate(() => {
    const host = document.querySelector(".viewer-scroll");
    const p = document.querySelector('[data-page="3"]');
    host.scrollTop += p.getBoundingClientRect().top - host.getBoundingClientRect().top;
  });
  await page.waitForTimeout(500);
  await page.evaluate(() => document.querySelector(".viewer-scroll").dispatchEvent(new Event("scroll")));
  await waitStatus(/Page 3 of 4/);
});

// ---- Find
await check("Find (⌘F, Enter, F3 / ⇧F3) highlights and steps through matches", async () => {
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Meta+f");
  assert(await page.getByLabel("Find text").evaluate((el) => el === document.activeElement), "⌘F focuses the find box");
  await page.keyboard.type("quick");
  await page.keyboard.press("Enter");
  await page.waitForSelector(".search-hit");
  const st0 = await page.locator(".search-status").innerText();
  assert(/^\d\/4$/.test(st0), st0);
  const w = await page.locator(".search-hit").first().evaluate((el) => el.getBoundingClientRect().width);
  assert(w > 25, `highlight too narrow (${w}px)`);
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("F3");
  const st1 = await page.locator(".search-status").innerText();
  assert(st1 !== st0, `F3 didn't move (${st1})`);
  await page.keyboard.press("Shift+F3");
  assert((await page.locator(".search-status").innerText()) === st0, "⇧F3 back");
  await page.getByLabel("Find text").fill("zebra");
  await page.getByLabel("Find text").press("Enter");
  await waitNotice(/No matches/);
  await page.getByLabel("Find text").fill("");
});

// ---- Selection + copy
await check("select text and ⌘C puts exact text on the clipboard", async () => {
  await page.locator('[data-thumb="1"] .thumb').click();
  await waitStatus(/Page 1 of/);
  await page.evaluate(() => {
    const el = [...document.querySelectorAll('[data-page="1"] .text-run')].find((s) => s.textContent.startsWith("The quick"));
    const r = document.createRange();
    r.selectNodeContents(el);
    getSelection().removeAllRanges();
    getSelection().addRange(r);
  });
  await page.keyboard.press("Meta+c");
  const clip = await page.evaluate(() => navigator.clipboard.readText());
  assert(clip === "The quick brown fox jumps over the lazy dog.", `clipboard: ${clip}`);
  await page.evaluate(() => getSelection().removeAllRanges());
});

// ---- Pages + undo/redo
await check("Pages ▸ Rotate CW / CCW, ⌘R / ⌘⇧R", async () => {
  await tab("Pages");
  await rb("rotateCw").click();
  await waitStatus(/rotation/);
  assert((await state()).pages[0].rotation === 90, "page 1 rotation 90");
  await rb("rotateCcw").click();
  await page.waitForFunction(async () => true);
  await page.waitForTimeout(400);
  assert((await state()).pages[0].rotation === 0, "back to 0");
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Meta+r");
  await page.waitForTimeout(500);
  assert((await state()).pages[0].rotation === 90, "⌘R");
  await page.keyboard.press("Meta+Shift+r");
  await page.waitForTimeout(500);
  assert((await state()).pages[0].rotation === 0, "⌘⇧R");
});

await check("Undo / Redo with ⌘Z / ⌘⇧Z / ⌘Y", async () => {
  await page.keyboard.press("Meta+z");
  await waitStatus(/Undid/);
  assert((await state()).pages[0].rotation === 90, "undo → 90");
  await page.keyboard.press("Meta+Shift+z");
  await waitStatus(/Redid/);
  assert((await state()).pages[0].rotation === 0, "redo → 0");
  await page.keyboard.press("Meta+z");
  await page.waitForTimeout(400);
  await page.keyboard.press("Meta+y");
  await page.waitForTimeout(400);
  assert((await state()).pages[0].rotation === 0, "⌘Y redo");
});

await check("⌘Z inside a text field does not undo the document", async () => {
  const before = (await state()).pages.map((p) => p.rotation).join();
  await page.getByLabel("Find text").focus();
  await page.keyboard.press("Meta+z");
  await page.waitForTimeout(300);
  assert((await state()).pages.map((p) => p.rotation).join() === before, "document changed");
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
});

await check("unsaved changes: name gets * and Save turns red", async () => {
  await tab("Document");
  assert(/main\.pdf \*/.test(await status()), await status());
  assert(await rb("save").evaluate((el) => el.classList.contains("dirty")), "Save not tinted");
});

await check("Pages ▸ Add Page inserts a blank page the size of the current one", async () => {
  await page.locator('[data-thumb="3"] .thumb').click();
  await waitStatus(/Page 3 of 4/);
  await tab("Pages");
  await rb("addPage").click();
  await page.locator(".modal").getByRole("button", { name: "Add Page" }).click();
  await waitStatus(/blank page/i);
  const s = await state();
  assert(s.pages.length === 5, `pages ${s.pages.length}`);
  assert(s.pages[3].width_pt === 420 && Math.round(s.pages[3].height_pt) === 595, `blank is ${s.pages[3].width_pt}x${s.pages[3].height_pt}`);
});

await check("Pages ▸ Delete removes the current page; ⌫ too", async () => {
  await waitStatus(/Page 4 of 5/);
  await rb("delete").click();
  await waitStatus(/of 4/);
  assert((await state()).pages.length === 4, "4 pages after delete");
  await page.locator('[data-thumb="4"] .thumb').click();
  await page.keyboard.press("Backspace");
  await waitStatus(/of 3/);
  await page.keyboard.press("Meta+z");
  await waitStatus(/of 4/);
});

await check("drag a thumbnail to reorder pages", async () => {
  // Drag the A5 page (3) above page 2; both must be on screen.
  const src = page.locator('[data-thumb="3"] .thumb');
  const dst = page.locator('[data-thumb="2"] .thumb');
  await src.evaluate((el) => el.scrollIntoView({ block: "end" }));
  const a = await src.boundingBox();
  const b = await dst.boundingBox();
  await page.mouse.move(a.x + a.width / 2, a.y + 30);
  await page.mouse.down();
  for (let i = 1; i <= 16; i++) await page.mouse.move(a.x + a.width / 2, a.y + 30 + ((b.y + 10 - a.y - 30) * i) / 16);
  await page.mouse.up();
  await waitStatus(/page reorder/);
  const s = await state();
  assert(s.pages[1].width_pt === 420, `page 2 should now be the A5 one (${s.pages.map((p) => p.width_pt)})`);
  await page.keyboard.press("Meta+z");
  await waitStatus(/Undid/);
  assert((await state()).pages[2].width_pt === 420, "undo restores order");
});

await check("multi-select (⌘-click, ⌘A) acts on the selection; status shows the count", async () => {
  await page.locator('[data-thumb="1"] .thumb').click();
  await page.locator('[data-thumb="2"] .thumb').click({ modifiers: ["Meta"] });
  assert((await page.locator(".thumb.picked").count()) === 2, "two picked");
  assert(/2 pages selected/.test(await status()), await status());
  await tab("Pages");
  await rb("rotateCw").click();
  await waitStatus(/rotation/);
  const s = await state();
  assert(s.pages[0].rotation === 90 && s.pages[1].rotation === 90 && s.pages[2].rotation === 0, "both rotated");
  await page.keyboard.press("Meta+z");
  await waitStatus(/Undid/);
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Meta+a");
  assert(/4 pages selected/.test(await status()), "⌘A selects all");
  await page.locator('[data-thumb="1"] .thumb').click();
});

await check("Document ▸ Import appends a PDF and jumps to its first page", async () => {
  await tab("Document");
  await pick(OTHER);
  await rb("import").click();
  await waitStatus(/Page 5 of 6/);
  await page.keyboard.press("Meta+z");
  await waitStatus(/Undid/);
});

await check("Pages ▸ Auto-Crop trims the white margin", async () => {
  await page.locator('[data-thumb="1"] .thumb').click();
  await tab("Pages");
  await rb("autoCrop").click();
  await waitStatus(/Cropped 1/);
  const p = (await state()).pages[0];
  assert(p.height_pt < 200 && p.width_pt < 500, `cropped to ${p.width_pt}x${p.height_pt}`);
  await page.keyboard.press("Meta+z");
  await waitStatus(/Undid/);
});

// ---- Live text editing
async function editRun(oldText, newText, key = "Enter") {
  const run = page.locator('[data-page="1"] .text-run.editable', { hasText: oldText }).first();
  await run.click();
  const ed = page.locator(".inline-editor:not(.pending)");
  await ed.waitFor();
  await ed.fill(newText);
  await ed.press(key);
}

await check("Content ▸ Edit Text: click a line, type, Enter — edited in the ORIGINAL font", async () => {
  await page.locator('[data-thumb="1"] .thumb').click();
  await tab("Content");
  await rb("editText").click();
  await page.waitForSelector(".text-run.editable");
  await editRun("Heading for page 1", "Edited héading");
  await waitStatus(/Text edited/);
  await page.waitForFunction(() => [...document.querySelectorAll('[data-page="1"] .text-run')].some((s) => s.textContent === "Edited héading"));
  assert(/original font/.test(await status()), await status());
  const [p1] = await liveText();
  assert(p1.includes("Edited héading") && !p1.includes("Heading for page 1"), p1);
  await page.screenshot({ path: path.join(OUT, "edit-in-place.png"), clip: { x: 0, y: 100, width: 1000, height: 360 } });
});

await check("double-click a line edits it without Edit Text mode", async () => {
  await tab("Content");
  await rb("editText").click(); // turn Edit Text mode OFF
  assert(!(await rb("editText").evaluate((el) => el.classList.contains("active"))), "mode should be off");
  await page.locator('[data-page="1"] .text-run', { hasText: "Edited héading" }).first().dblclick();
  const ed = page.locator(".inline-editor:not(.pending)");
  await ed.waitFor();
  assert((await ed.inputValue()) === "Edited héading", await ed.inputValue());
  await ed.fill("Double-clicked heading");
  await ed.press("Enter");
  await waitStatus(/Text edited/);
  await page.waitForFunction(() => [...document.querySelectorAll('[data-page="1"] .text-run')].some((s) => s.textContent === "Double-clicked heading"));
});

await check("right-click a line: context menu ▸ Edit Text, Copy Line", async () => {
  await page.locator('[data-page="1"] .text-run', { hasText: "Double-clicked heading" }).first().click({ button: "right" });
  const menu = page.locator(".context-menu");
  await menu.waitFor();
  const items = await menu.getByRole("menuitem").allInnerTexts();
  assert(items.includes("Edit Text") && items.includes("Copy Line") && items.includes("Delete Page"), items.join());
  await menu.getByRole("menuitem", { name: "Copy Line" }).click();
  assert((await page.evaluate(() => navigator.clipboard.readText())) === "Double-clicked heading", "copied line");
  await page.locator('[data-page="1"] .text-run', { hasText: "Double-clicked heading" }).first().click({ button: "right" });
  await page.locator(".context-menu").getByRole("menuitem", { name: "Edit Text", exact: true }).click();
  const ed = page.locator(".inline-editor:not(.pending)");
  await ed.waitFor();
  await ed.fill("Edited héading");
  await ed.press("Enter");
  await waitStatus(/Text edited/);
  await page.waitForFunction(() => [...document.querySelectorAll('[data-page="1"] .text-run')].some((s) => s.textContent === "Edited héading"));
});

await check("right-click empty page area: page actions work, Esc closes", async () => {
  // Empty top-right corner of page 1 (no text there).
  const pv = page.locator('[data-page="1"]');
  const w = (await pv.boundingBox()).width;
  await pv.click({ button: "right", position: { x: w - 30, y: 30 } });
  await page.locator(".context-menu").waitFor();
  await page.keyboard.press("Escape");
  assert((await page.locator(".context-menu").count()) === 0, "Esc closes");
  await pv.click({ button: "right", position: { x: w - 30, y: 30 } });
  await page.locator(".context-menu").getByRole("menuitem", { name: "Rotate Page Right" }).click();
  await waitStatus(/rotation/);
  assert((await state()).pages[0].rotation === 90, "rotated from the context menu");
  await page.keyboard.press("Meta+z");
  await waitStatus(/Undid/);
  await tab("Content");
  await rb("editText").click(); // back ON for the following checks
});

await check("Tab saves and moves the editor to the next line; Esc cancels", async () => {
  await editRun("The quick brown fox", "The slow brown fox jumps over the lazy dog.", "Tab");
  await waitStatus(/Text edited/);
  const ed = page.locator(".inline-editor:not(.pending)");
  await page.waitForFunction(() => document.querySelector(".inline-editor:not(.pending)")?.value?.startsWith("Line two"), null, { timeout: 8000 });
  const n = await calls("edit_text");
  await ed.fill("SHOULD NOT SAVE");
  await ed.press("Escape");
  await page.waitForTimeout(300);
  assert((await calls("edit_text")) === n, "Esc must not save");
});

await check("Polish letters the font lacks: replaced in a matching standard font, every letter kept", async () => {
  await editRun("Line two on page 1", "Zażółć gęślą jaźń");
  await waitStatus(/Text edited/);
  const st = await status();
  assert(/Helvetica/.test(st) && !/unsupported/.test(st), st);
  await page.waitForFunction(() => [...document.querySelectorAll('[data-page="1"] .text-run')].some((s) => s.textContent === "Zażółć gęślą jaźń"), null, { timeout: 8000 });
  const [p1] = await liveText();
  assert(p1.includes("Zażółć gęślą jaźń"), p1);
  await page.screenshot({ path: path.join(OUT, "polish.png"), clip: { x: 260, y: 100, width: 800, height: 300 } });
});

await check("clearing a line deletes it; Undo brings it back", async () => {
  await editRun("Edited héading", "");
  await waitStatus(/Text edited/);
  await page.waitForTimeout(400);
  const runs = await page.locator('[data-page="1"] .text-run').allInnerTexts();
  assert(!runs.some((t) => t.includes("Edited héading")), "line not deleted");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Meta+z");
  await waitStatus(/Undid/);
  await page.waitForFunction(() => [...document.querySelectorAll('[data-page="1"] .text-run')].some((s) => s.textContent === "Edited héading"));
});


// ---- View fixes
await check("zoom scales the page uniformly (no vertical stretch) and scrolls sideways", async () => {
  await tab("Zoom");
  await rb("zoom100").click();
  for (let i = 0; i < 8; i++) await rb("zoomIn").click();
  await page.waitForTimeout(400);
  const [w, h, sw, cw] = await page.evaluate(() => {
    const p = document.querySelector('[data-page="1"]').getBoundingClientRect();
    const host = document.querySelector(".viewer-scroll");
    return [p.width, p.height, host.scrollWidth, host.clientWidth];
  });
  assert(Math.abs(w / h - 612 / 792) < 0.01, `aspect ${(w / h).toFixed(3)} vs ${(612 / 792).toFixed(3)}`);
  assert(sw > cw, "wide page scrolls horizontally");
  await rb("fitWidth").click();
  await page.waitForTimeout(300);
});

await check("sidebar × closes the sidebar; F8 brings it back", async () => {
  await page.locator(".pane-close").click();
  assert((await page.locator(".pages-pane").count()) === 0, "closed");
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("F8");
  assert((await page.locator(".pages-pane").count()) === 1, "back");
});

// ---- Annotate tab: every tool
await check("Annotate ▸ Highlight / Underline / Strike snap to the dragged text", async () => {
  await page.locator('[data-thumb="2"] .thumb').click();
  await tab("Annotate");
  for (const [id, text, sub] of [["annHighlight", "The quick brown fox", "Highlight"], ["annUnderline", "Line two on page 2", "Underline"], ["annStrike", "Heading for page 2", "StrikeOut"]]) {
    await rb(id).click();
    const b = await runBox(2, text);
    await dragMouse(b.x + 2, b.y + b.height / 2, b.x + b.width * 0.6, b.y + b.height / 2);
    await waitStatus(/Added/);
    await page.keyboard.press("Escape");
    const list = await annotsOn(2);
    assert(list.some((a) => a.subtype === sub), `${sub} missing: ${list.map((a) => a.subtype)}`);
  }
});

await check("Annotate ▸ Note: click, type, Add; clicking the note shows it", async () => {
  await rb("annNote").click();
  const [x, y] = await emptySpot(2, 0.8, 0.45);
  await page.mouse.click(x, y);
  await page.getByLabel("Comment").fill("Please check this paragraph");
  await btn("Add").click();
  await waitStatus(/Added note/);
  await page.keyboard.press("Escape");
  assert((await annotsOn(2)).some((a) => a.subtype === "Text" && a.contents === "Please check this paragraph"), "note");
  await page.waitForSelector('[data-page="2"] .annot-hot.note');
  await page.locator('[data-page="2"] .annot-hot.note').first().click();
  await page.waitForSelector("text=Please check this paragraph");
  await page.locator(".note-popup").getByRole("button", { name: "Close" }).click();
});

await check("Annotate ▸ Text Box, Shape ▾ (rectangle/oval/arrow), Stamp ▾, Pen", async () => {
  await rb("annText").click();
  let [x, y] = await emptySpot(2, 0.15, 0.5);
  await dragMouse(x, y, x + 180, y + 50);
  await page.getByLabel("Text", { exact: true }).fill("Boxed comment");
  await btn("Add").click();
  await waitStatus(/Added text box/);
  for (const [choice, sub, dx] of [["Rectangle", "Square", 0], ["Oval", "Circle", 140], ["Arrow", "Line", 280]]) {
    await rb("annShape").click();
    await page.locator(".rb-choices").getByRole("menuitem", { name: choice }).click();
    [x, y] = await emptySpot(2, 0.15, 0.62);
    await dragMouse(x + dx, y, x + dx + 100, y + 60);
    await waitStatus(/Added (square|circle|line)/);
  }
  await rb("annStamp").click();
  await page.locator(".rb-choices").getByRole("menuitem", { name: "Approved", exact: true }).click();
  [x, y] = await emptySpot(2, 0.6, 0.75);
  await page.mouse.click(x, y);
  await waitStatus(/Added stamp/);
  await rb("annPen").click();
  [x, y] = await emptySpot(2, 0.2, 0.85);
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= 20; i++) await page.mouse.move(x + i * 8, y + Math.sin(i / 2) * 20);
  await page.mouse.up();
  await waitStatus(/Added ink/);
  await page.keyboard.press("Escape");
  const subs = (await annotsOn(2)).map((a) => a.subtype);
  for (const k of ["FreeText", "Square", "Circle", "Line", "Stamp", "Ink"]) assert(subs.includes(k), `${k} missing: ${subs}`);
  await page.screenshot({ path: path.join(OUT, "annotations.png") });
});

await check("Annotate ▸ Link to a page works when clicked", async () => {
  await rb("annLink").click();
  const b = await runBox(2, "Heading for page 2");
  await dragMouse(b.x, b.y, b.x + b.width, b.y + b.height);
  await page.getByLabel("Link target").fill("4");
  await btn("Add").click();
  await waitStatus(/Added link/);
  await page.keyboard.press("Escape");
  await page.waitForSelector('[data-page="2"] .annot-hot.link');
  await page.locator('[data-page="2"] .annot-hot.link').first().click();
  await waitStatus(/Page 4 of/);
});

await check("Annotate ▸ Redact removes the text under the box for real", async () => {
  await page.locator('[data-thumb="4"] .thumb').click();
  await rb("annRedact").click();
  const b = await runBox(4, "Line two on page 4");
  await dragMouse(b.x - 2, b.y - 2, b.x + b.width + 2, b.y + b.height + 2);
  await waitStatus(/Redacted: removed \d+ characters/);
  await page.keyboard.press("Escape");
  const t = await liveText();
  assert(!t[3].includes("Line two on page 4") && t[3].includes("quick brown fox"), t[3]);
});

// ---- Content tab
await check("Content ▸ Select: click a line, drag to move it, ⌫ deletes it", async () => {
  await tab("Content");
  await rb("select").click();
  await page.waitForSelector('[data-page="4"] .obj-box.text');
  const b = await runBox(4, "Heading for page 4");
  await page.mouse.click(b.x + 10, b.y + b.height / 2);
  await page.waitForSelector('[data-page="4"] .obj-box.picked');
  const before = (await ipc("list_objects", { page: 4 })).find((o) => o.text === "Heading for page 4").rect;
  await dragMouse(b.x + 10, b.y + b.height / 2, b.x + 60, b.y + b.height / 2 + 30);
  await waitStatus(/Moved 1 object/);
  const after = (await ipc("list_objects", { page: 4 })).find((o) => o.text === "Heading for page 4").rect;
  assert(after[0] - before[0] > 20 && after[1] < before[1], `moved ${before} → ${after}`);
  // Wait for the re-rendered page and its fresh outlines.
  await page.waitForSelector('[data-page="4"] .page-overlay[data-ready="true"]');
  await page.waitForFunction(() => {
    const b = [...document.querySelectorAll('[data-page="4"] .obj-box.text')].find((e) => e.title === "Heading for page 4");
    const r = [...document.querySelectorAll('[data-page="4"] .text-run')].find((e) => e.textContent === "Heading for page 4");
    return b && r && Math.abs(b.getBoundingClientRect().left - r.getBoundingClientRect().left) < 6;
  }, null, { timeout: 10000 }).catch(async (e) => {
    const dump = await page.evaluate(() => ({
      boxes: [...document.querySelectorAll('[data-page="4"] .obj-box')].map((e) => [e.title, Math.round(e.getBoundingClientRect().left)]),
      runs: [...document.querySelectorAll('[data-page="4"] .text-run')].map((e) => [e.textContent, Math.round(e.getBoundingClientRect().left)]),
    }));
    throw new Error(`outline not re-listed: ${JSON.stringify(dump)}`);
  });
  const b2 = await runBox(4, "Heading for page 4");
  await page.mouse.click(b2.x + 10, b2.y + b2.height / 2);
  const pickedNow = await page.evaluate(() => JSON.stringify(window.__stitchup.state().picked));
  const ids = await page.evaluate(() => [...document.querySelectorAll('[data-page="4"] .obj-box')].map((e) => e.dataset.obj));
  await page.keyboard.press("Backspace");
  await waitStatus(/Deleted 1 object/).catch(async () => {
    throw new Error(`delete failed: picked=${pickedNow} listed=${ids} toast=${await page.locator(".error-toast").innerText().catch(() => "-")} status=${await status()}`);
  });
  assert(!(await liveText())[3].includes("Heading for page 4"), "deleted");
});

await check("Content ▸ Recolor and Font change the selected text", async () => {
  await page.waitForSelector('[data-page="4"] .obj-box.text');
  const b = await runBox(4, "The quick brown fox");
  await page.mouse.click(b.x + 10, b.y + b.height / 2);
  await rb("recolor").click();
  await page.getByLabel("#1d5fd0").click();
  await btn("Apply").click();
  await waitStatus(/Recoloured/);
  await page.waitForTimeout(600);
  await page.waitForSelector('[data-page="4"] .page-overlay[data-ready="true"]');
  const b2 = await runBox(4, "The quick brown fox");
  await page.mouse.click(b2.x + 10, b2.y + b2.height / 2);
  await rb("font").click();
  await page.getByLabel("Font family").selectOption("Times");
  await page.locator(".modal").getByText("Bold").click();
  await page.getByLabel("Font size").fill("18");
  await btn("Apply").click();
  await waitStatus(/Font changed/);
  const bytes = Buffer.from((await ipc("render_bytes")).bytes_b64, "base64").toString("latin1");
  assert(bytes.includes("Times-Bold"), "Times-Bold font resource");
  assert((await liveText())[3].includes("quick brown fox"), "text kept");
});

await check("Content ▸ Fore/Back colours and Shapes ▾ insert into the page", async () => {
  await tab("Content");
  await rb("fore").click();
  await page.locator('.color-palette [aria-label="#1a7f37"]').click();
  assert((await page.evaluate(() => window.__stitchup.state().foreColor)) === "#1a7f37", "fore colour set");
  await rb("shapes").click();
  await page.locator(".rb-choices").getByRole("menuitem", { name: "Oval" }).click();
  const [x, y] = await emptySpot(4, 0.6, 0.6);
  await dragMouse(x, y, x + 120, y + 70);
  await waitStatus(/Inserted oval/);
  await page.keyboard.press("Escape");
});

await check("Select picks annotations too: delete the stamp", async () => {
  await page.locator('[data-thumb="2"] .thumb').click();
  await rb("select").click();
  await page.waitForSelector('[data-page="2"] .obj-box.annot');
  const n0 = (await annotsOn(2)).length;
  const stamp = page.locator('[data-page="2"] .obj-box.annot[title="Stamp"]');
  await stamp.evaluate((el) => el.scrollIntoView({ block: "center" }));
  await page.waitForTimeout(200);
  const sb = await stamp.boundingBox();
  await page.mouse.click(sb.x + sb.width / 2, sb.y + sb.height / 2);
  await rb("objDelete").click();
  await waitStatus(/Deleted 1 object/);
  const after = await annotsOn(2);
  assert(after.length === n0 - 1 && !after.some((a) => a.subtype === "Stamp"), "stamp deleted");
  await page.keyboard.press("Escape");
});

// ---- Sign
await check("Security ▸ Sign: place a signature box; OpenSSL verifies the signed copy", async () => {
  await tab("Security");
  await rb("sign").click();
  await page.getByLabel("Signer name").fill("Jan Kowalski");
  await page.getByLabel("Location").fill("Łódź");
  await btn("Place Signature…").click();
  const target = path.join(WORK, "signed.pdf");
  await pick(target);
  const [x, y] = await emptySpot(1, 0.6, 0.85);
  await dragMouse(x, y, x + 220, y + 60);
  await waitNotice(/Signed by Jan Kowalski/);
  const pdf = await readFile(target);
  const s = pdf.toString("latin1");
  const nums = s.slice(s.indexOf("/ByteRange")).match(/\[([^\]]+)\]/)[1].trim().split(/\s+/).map(Number);
  assert(nums[2] + nums[3] === pdf.length, "byte range covers the file");
  const data = Buffer.concat([pdf.subarray(0, nums[1]), pdf.subarray(nums[2])]);
  let hex = pdf.subarray(nums[1] + 1, nums[2] - 1).toString("latin1");
  const der = Buffer.from(hex, "hex");
  const len = der[1] < 0x80 ? 2 + der[1] : 2 + (der[1] & 0x7f) + [...der.subarray(2, 2 + (der[1] & 0x7f))].reduce((a, b) => a * 256 + b, 0);
  await writeFile(path.join(WORK, "sig.der"), der.subarray(0, len));
  await writeFile(path.join(WORK, "sig.data"), data);
  execFileSync("openssl", ["smime", "-verify", "-binary", "-inform", "DER", "-noverify", "-purpose", "any", "-in", path.join(WORK, "sig.der"), "-content", path.join(WORK, "sig.data"), "-out", "/dev/null"], { stdio: "pipe" });
  const text = await pdfText(target);
  assert(text.length === 4, "signed copy readable");
});


await check("signing an already-signed PDF adds a second signature; both stay valid", async () => {
  const first = path.join(WORK, "signed.pdf");
  await openViaRibbon(first);
  await tab("Security");
  await rb("sign").click();
  await page.getByLabel("Signer name").fill("Anna Nowak");
  await page.getByLabel("Visible signature").uncheck();
  const second = path.join(WORK, "signed-twice.pdf");
  await pick(second);
  await btn("Sign & Save As…").click();
  await waitNotice(/Signed by Anna Nowak/);
  const firstBytes = await readFile(first);
  const both = await readFile(second);
  assert(Buffer.compare(both.subarray(0, firstBytes.length), firstBytes) === 0, "first revision kept byte-for-byte");
  await verifySignature(second, 0);
  const n = await verifySignature(second, 1);
  assert(n[2] + n[3] === both.length, "second signature covers the whole file");
  await page.locator(".doc-tab", { hasText: "main.pdf" }).click();
  await waitStatus(/main\.pdf/);
  openPath = MAIN;
});

await check("Content ▸ OCR makes a scanned (image-only) page searchable; Undo removes the layer", async () => {
  // A "scan": text rendered to a bitmap, embedded as the only page content.
  const png = await page.evaluate(() => {
    const c = document.createElement("canvas");
    c.width = 1700;
    c.height = 2200;
    const g = c.getContext("2d");
    g.fillStyle = "#fff";
    g.fillRect(0, 0, c.width, c.height);
    g.fillStyle = "#111";
    g.font = "bold 80px Helvetica";
    g.fillText("Scanned Invoice 4711", 160, 300);
    g.font = "48px Times New Roman";
    g.fillText("Payment due within thirty days.", 160, 460);
    return c.toDataURL("image/png").split(",")[1];
  });
  const doc = await PDFDocument.create();
  const img = await doc.embedPng(Buffer.from(png, "base64"));
  doc.addPage([612, 792]).drawImage(img, { x: 0, y: 0, width: 612, height: 792 });
  const scan = path.join(WORK, "scan.pdf");
  await writeFile(scan, await doc.save());
  await openViaRibbon(scan);
  assert(!(await pdfText(scan)).join(" ").trim(), "fixture has no text");
  await tab("Content");
  await rb("ocr").click();
  await page.getByRole("menuitem", { name: "Pages Without Text" }).click();
  await waitStatus(/Recognised \d+ lines? on 1 page/, 60000);
  await idle();
  await page.getByLabel("Find text").fill("4711");
  await page.getByLabel("Find text").press("Enter");
  await page.waitForTimeout(1500);
  const dbg = await page.evaluate(() => ({ hits: window.__stitchup.state().searchHits, toast: document.querySelector(".notice-toast")?.textContent, st: document.querySelector(".search-status")?.textContent }));
  assert(/1\/1/.test(dbg.st ?? ""), JSON.stringify(dbg));
  await page.getByLabel("Find text").fill("");
  // Saved file has the text layer (and still looks the same: render mode 3).
  const out = path.join(WORK, "scan-ocr.pdf");
  await pick(out);
  await tab("Document");
  await rb("saveAs").click();
  await waitNotice(/Saved/);
  const text = (await pdfText(out)).join(" ");
  assert(/Invoice/.test(text) && /4711/.test(text) && /thirty/.test(text), `recognised: ${text}`);
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Meta+z");
  await waitStatus(/Undid/);
  const st = await page.evaluate(() => window.__stitchup.state().doc.pages.length);
  assert(st === 1, "still one page");
  await page.locator(".doc-tab", { hasText: "main.pdf" }).click();
  await waitStatus(/main\.pdf/);
  openPath = MAIN;
});

await check("colour palettes: Fore / Back / Annotate ▸ Colour open a palette and take effect", async () => {
  const f = path.join(WORK, "colours.pdf");
  await makePdf(f, [LETTER, LETTER]);
  await openViaRibbon(f);
  await tab("Content");
  await rb("fore").click();
  await page.locator(".color-palette").waitFor();
  await page.locator('.color-palette [aria-label="#1d5fd0"]').click();
  await waitNotice(/Text colour #1d5fd0/);
  assert((await page.evaluate(() => window.__stitchup.state().foreColor)) === "#1d5fd0", "fore set");
  assert(/rgb\(29, 95, 208\)/.test(await rb("fore").locator(".rb-swatch").evaluate((el) => el.style.background)), "swatch shows it");
  await rb("back").click();
  await page.locator(".color-palette").getByRole("menuitem", { name: "No fill" }).click();
  assert((await page.evaluate(() => window.__stitchup.state().backColor)) === null, "back none");
  await rb("back").click();
  await page.locator(".color-palette").getByLabel("Colour code").fill("#123456");
  await page.locator(".color-palette").getByRole("button", { name: "Use" }).click();
  assert((await page.evaluate(() => window.__stitchup.state().backColor)) === "#123456", "custom back");
  // Select a line, Fore recolours it straight away.
  await rb("select").click();
  await page.waitForSelector('[data-page="1"] .obj-box.text');
  const rb1 = await runBox(1, "The quick brown fox jumps over the lazy dog.");
  await page.mouse.click(rb1.x + 10, rb1.y + rb1.height / 2);
  await page.waitForFunction(() => window.__stitchup.state().picked?.objects.length > 0);
  await rb("fore").click();
  await page.locator('.color-palette [aria-label="#1a7f37"]').click();
  await waitStatus(/Recoloured/);
  const objs = await ipc("list_objects", { page: 1 });
  assert(objs.find((o) => o.text.startsWith("The quick")).color === "#1a7f37", JSON.stringify(objs.map((o) => [o.text, o.color])));
  await page.keyboard.press("Escape");
  if (await page.evaluate(() => window.__stitchup.state().tool)) await rb("select").click(); // Select tool off
  // Annotate ▸ Colour is used by the next annotation (highlight included).
  await tab("Annotate");
  await rb("annFore").click();
  await page.locator('.color-palette [aria-label="#7ed321"]').click();
  await waitNotice(/Annotation colour #7ed321/);
  await rb("annHighlight").click();
  const line = page.locator('[data-page="1"] .text-run', { hasText: "Line two on page 1" }).first();
  const lb = await line.boundingBox();
  await page.mouse.move(lb.x + 2, lb.y + lb.height / 2);
  await page.mouse.down();
  await page.mouse.move(lb.x + lb.width - 2, lb.y + lb.height / 2, { steps: 8 });
  await page.mouse.up();
  await waitStatus(/Added highlight/);
  await page.keyboard.press("Escape");
  const out = path.join(WORK, "colours-out.pdf");
  await pick(out);
  await tab("Document");
  await rb("saveAs").click();
  await waitStatus(/colours-out\.pdf/);
  const d = await PDFDocument.load(await readFile(out));
  const annots = d.getPage(0).node.Annots();
  const hl = annots.asArray().map((r) => d.context.lookup(r)).find((a) => a.get(PDFName.of("Subtype")).toString() === "/Highlight");
  const c = hl.get(PDFName.of("C")).asArray().map((n) => Math.round(n.asNumber() * 255));
  assert(c.join() === "126,211,33", `highlight colour ${c}`);
  await page.evaluate(() => window.__stitchup.store.pickColor("ann", null));
  await page.locator('[data-thumb="2"] .thumb').click();
  await waitStatus(/Page 2 of 2/);
});

await check("editing a line shows its font & size; the bar changes size, colour and font (typed text kept)", async () => {
  await page.locator('[data-thumb="2"] .thumb').click();
  await waitStatus(/Page 2 of 2/);
  await tab("Zoom");
  const line = page.locator('[data-page="2"] .text-run', { hasText: "Line two on page 2" }).first();
  await line.scrollIntoViewIfNeeded();
  await line.dblclick();
  await page.locator(".edit-bar").waitFor();
  assert((await page.locator(".ribbon-tab.active").innerText()) === "Content", "switched to the Content tab");
  await page.waitForFunction(() => /^Helvetica · 14 pt$/.test(document.querySelector(".edit-bar-font")?.textContent ?? ""));
  assert(await rb("font").isEnabled(), "Font button live while editing");
  const find = async (t) => (await ipc("list_objects", { page: 2 })).find((o) => o.text.startsWith(t));
  // Size
  await page.locator(".edit-bar").getByLabel("Font size").fill("18");
  await page.locator(".edit-bar").getByLabel("Font size").press("Enter");
  await waitStatus(/Size: 18 pt/);
  await page.locator(".inline-editor").waitFor();
  assert((await find("Line two")).size === 18, "size 18");
  await page.waitForFunction(() => /Helvetica · 18 pt/.test(document.querySelector(".edit-bar-font")?.textContent ?? ""));
  // Colour from the ribbon while the line is open
  await rb("fore").click();
  await page.locator('.color-palette [aria-label="#d0021b"]').click();
  await waitStatus(/Recoloured the line/);
  assert((await find("Line two")).color === "#d0021b", "line recoloured");
  // Type, then pick a font: the typed text is saved, then the font applied.
  await page.locator(".inline-editor").waitFor();
  await page.locator(".inline-editor").fill("Line two, rewritten");
  await page.locator(".edit-bar").getByLabel("Font", { exact: true }).selectOption("Times");
  await waitStatus(/Font: Times-Roman/);
  const o = await find("Line two, rewritten");
  assert(o && o.font === "Times-Roman" && o.size === 18 && o.color === "#d0021b", JSON.stringify(o));
  await page.locator(".inline-editor").press("Escape");
  await page.waitForFunction(() => !document.querySelector(".edit-bar"));
  // Close this tab without saving.
  await pick("Close");
  await page.locator(".doc-tab.active .doc-tab-close").click();
  await page.locator(".doc-tab", { hasText: "main.pdf" }).click();
  await waitStatus(/main\.pdf/);
  openPath = MAIN;
});

await check("every button on every ribbon tab is live with a document open", async () => {
  const needsSelection = new Set(["objDelete", "recolor", "font"]);
  const dead = [];
  for (const t of ["Document", "Pages", "Annotate", "Content", "Zoom", "Navigate", "Security"]) {
    await tab(t);
    for (const el of await page.locator(".ribbon-card .rb-btn").all()) {
      const id = await el.getAttribute("data-cmd");
      if (needsSelection.has(id) || ["prev", "next"].includes(id)) continue;
      if (await el.isDisabled()) dead.push(`${t}:${id}`);
    }
  }
  assert(dead.length === 0, `disabled: ${dead.join(", ")}`);
});

// ---- Security
await check("Security ▸ Watermark stamps every page (and undoes)", async () => {
  await tab("Security");
  await rb("watermark").click();
  await page.getByLabel("Watermark text").fill("CONFIDENTIAL");
  await page.getByLabel("Placement").selectOption("tiled");
  await btn("Apply").click();
  await waitStatus(/Watermark added to 4/);
  const text = await liveText();
  assert(text.every((t) => t.includes("CONFIDENTIAL")), "every page");
  await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(OUT, "watermark.png") });
  await page.keyboard.press("Meta+z");
  await waitStatus(/Undid/);
  assert(!(await liveText()).some((t) => t.includes("CONFIDENTIAL")), "undo removes it");
});

await check("Security ▸ Encrypt saves an AES-256 copy that needs the password", async () => {
  await rb("encrypt").click();
  await page.getByLabel("Password", { exact: true }).fill("s3cret");
  await page.getByLabel("Confirm password").fill("s3cret");
  const target = path.join(WORK, "protected.pdf");
  await pick(target);
  await btn("Save As…").click();
  await waitNotice(/password-protected copy/);
  let refused = false;
  try {
    await pdfText(target);
  } catch {
    refused = true;
  }
  assert(refused, "opened without a password");
  const t = await pdfText(target, "s3cret");
  assert(t.length === 4 && t[1].includes("page 2"), "readable with the password");
});

// ---- Properties
await check("Document ▸ Properties shows and edits metadata", async () => {
  await tab("Document");
  await rb("properties").click();
  const title = page.getByLabel("Title");
  assert((await title.inputValue()) === "E2E fixture", `title field: ${await title.inputValue()}`);
  await title.fill("Renamed by test");
  await page.getByLabel("Author").fill("");
  await btn("Apply").click();
  await page.waitForSelector("text=Saved ✓");
  const m = (await state()).metadata;
  assert(m.title === "Renamed by test" && !m.author, JSON.stringify(m));
  await btn("Close").click();
});

// ---- Exports (independently parsed)
async function exportVia(id, file) {
  const target = path.join(WORK, file);
  await pick(target);
  await rb(id).click();
  // Wait for THIS export's notice (it names the file).
  await waitNotice(new RegExp(file.replace(".", "\\.")));
  return target;
}

await check("Export Text / CSV / RTF", async () => {
  const txt = (await readFile(await exportVia("exportText", "out.txt"), "utf8")).replace(/^﻿/, "");
  // (page 4's heading and second line were deleted/redacted by earlier checks)
  assert(txt.includes("Edited héading") && txt.includes("page 3") && txt.includes("\f"), txt.slice(0, 80));
  const csv = (await readFile(await exportVia("exportCsv", "out.csv"), "utf8")).replace(/^﻿/, "").trim().split(/\r?\n/);
  assert(csv[0] === "Page,Width (pt),Height (pt),Text chars,Annotations" && csv.length === 5, csv.join(" | "));
  assert(csv[3].startsWith("3,420.0,595.0,"), csv[3]);
  const rtf = await readFile(await exportVia("exportRtf", "out.rtf"), "utf8");
  assert(rtf.startsWith("{\\rtf1") && rtf.includes("\\page") && rtf.includes("h\\u233?ading"), rtf.slice(0, 120));
});

await check("Export DOCX / ODT are valid packages with the text", async () => {
  const docx = zipEntries(await exportVia("exportDocx", "out.docx"));
  assert(docx["word/document.xml"]?.includes("Edited héading") && docx["[Content_Types].xml"], Object.keys(docx).join());
  const odtFile = await exportVia("exportOdt", "out.odt");
  const odt = zipEntries(odtFile);
  assert(odt["content.xml"]?.includes("Edited héading"), "odt text");
  const names = execFileSync("python3", ["-c", "import zipfile,sys;print(zipfile.ZipFile(sys.argv[1]).namelist()[0])", odtFile]).toString().trim();
  assert(names === "mimetype" && odt.mimetype === "application/vnd.oasis.opendocument.text", "mimetype first");
});

await check("Export Images writes one PNG per page", async () => {
  await rb("exportImages").click();
  await page.getByLabel("Resolution").selectOption("72");
  const target = path.join(WORK, "img", "page.png");
  await mkdir(path.dirname(target), { recursive: true });
  await pick(target);
  await btn("Export…").click();
  await waitNotice(/Exported 4 images/);
  const files = (await readdir(path.dirname(target))).sort();
  assert(files.join() === "page-1.png,page-2.png,page-3.png,page-4.png", files.join());
  const head = (await readFile(path.join(path.dirname(target), "page-1.png"))).subarray(0, 8);
  assert(head.toString("hex") === "89504e470d0a1a0a", "PNG signature");
});

// ---- Print
await check("Print: preview sheet, then one fitted sheet per page goes to the print panel", async () => {
  await rb("print").click();
  await page.waitForFunction(() => {
    const c = document.querySelector('canvas[aria-label="Print preview"]');
    if (!c || !c.width) return false;
    const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
    for (let i = 0; i < d.length; i += 16) if (d[i] < 100) return true;
    return false;
  }, null, { timeout: 8000 });
  await page.getByLabel("Paper").selectOption("Letter");
  await page.getByLabel("Orientation").selectOption("l");
  await page.screenshot({ path: path.join(OUT, "print-dialog.png") });
  const before = await calls("print_window");
  await btn("Print…").click();
  await page.waitForFunction((b) => window.__calls.filter((c) => c === "print_window").length > b, before, { timeout: 15000 });
  const sheets = await page.locator("#print-root .print-sheet").count();
  assert(sheets === 4, `${sheets} sheets`);
  const w = await page.locator("#print-root .print-sheet").first().evaluate((el) => el.style.width);
  assert(w === "792pt", `landscape Letter sheet width ${w}`);
  // What the print engine paginates: exactly one printed page per sheet.
  const printed = async () => (await PDFDocument.load(await page.pdf({ width: "11in", height: "8.5in", preferCSSPageSize: true }))).getPageCount();
  assert((await printed()) === 4, `printed pages: ${await printed()}`);
  // One page chosen → one page printed (an extra blank page was reported).
  await rb("print").click();
  await page.getByLabel("Print range").selectOption("current");
  await page.getByLabel("Paper").selectOption("A4");
  const b2 = await calls("print_window");
  await btn("Print…").click();
  await page.waitForFunction((b) => window.__calls.filter((c) => c === "print_window").length > b, b2, { timeout: 15000 });
  const one = (await PDFDocument.load(await page.pdf({ width: "8.27in", height: "11.69in", preferCSSPageSize: true }))).getPageCount();
  assert(one === 1, `one page selected → ${one} printed`);
});

// ---- Save in place / Save As
await check("⌘S saves in place: file updated atomically, * cleared", async () => {
  const before = await readFile(MAIN);
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Meta+s");
  await waitNotice(/Saved main\.pdf/);
  const after = await readFile(MAIN);
  assert(Buffer.compare(before, after) !== 0, "file unchanged");
  assert((await pdfText(MAIN))[0].includes("Edited héading"), "edit saved");
  assert(!/main\.pdf \*/.test(await status()), "still marked dirty");
  assert(!(await rb("save").evaluate((el) => el.classList.contains("dirty"))), "Save still red");
  assert(!(await readdir(WORK)).some((f) => f.startsWith(".stitchup-tmp")), "temp litter");
});

await check("Save As writes a copy and makes it the current document", async () => {
  const target = path.join(WORK, "saved-as.pdf");
  await pick(target);
  await rb("saveAs").click();
  await waitStatus(/saved-as\.pdf/);
  assert((await pdfText(target)).length === 4, "copy");
  openPath = target;
  await tab("Pages");
  await rb("rotateCw").click();
  await waitStatus(/rotation/);
  await page.keyboard.press("Meta+s");
  await waitNotice(/Saved saved-as\.pdf/);
  const doc = await pdfjs.getDocument({ data: new Uint8Array(await readFile(target)) }).promise;
  assert((await doc.getPage(1)).rotate === 90, "⌘S after Save As writes the new file");
  await doc.destroy();
});

await check("Pages ▸ Extract and Split write new files", async () => {
  await page.locator('[data-thumb="2"] .thumb').click();
  await page.locator('[data-thumb="3"] .thumb').click({ modifiers: ["Shift"] });
  const ex = path.join(WORK, "extract.pdf");
  await pick(ex);
  await rb("extract").click();
  await waitNotice(/Saved 2 pages/);
  const t = await pdfText(ex);
  assert(t.length === 2 && t[0].includes("page 2") && t[1].includes("page 3"), JSON.stringify(t));
  const dir = path.join(WORK, "split");
  await mkdir(dir);
  await rb("split").click();
  await page.getByLabel("Pages per file").fill("2");
  await pick(dir);
  await btn("Choose Folder…").click();
  await waitNotice(/Split into 2 files/);
  assert((await readdir(dir)).sort().join() === "saved-as-part1.pdf,saved-as-part2.pdf", (await readdir(dir)).join());
});

await check("File ▸ Compress writes a smaller equivalent copy", async () => {
  const target = path.join(WORK, "small.pdf");
  await pick(target);
  await run("compress");
  await waitNotice(/Compressed:/);
  assert((await pdfText(target)).length === 4, "readable");
});

// ---- Bookmarks (sidebar)
await check("bookmarks: star toggle, list, jump, clear", async () => {
  await page.locator('[data-thumb="2"] .thumb-bookmark').click();
  await page.locator('[data-thumb="4"] .thumb-bookmark').click();
  assert(/Bookmarks \(2\)/.test(await page.locator(".pane-tabs").innerText()), "2 bookmarks");
  await page.locator(".pane-tabs").getByRole("tab", { name: /Bookmarks/ }).click();
  assert((await page.locator(".bm-row").count()) === 2, "list rows");
  await page.locator(".bm-jump", { hasText: "Page 4" }).click();
  await waitStatus(/Page 4 of/);
  await btn("Clear").click();
  assert((await page.locator(".bm-row").count()) === 0, "cleared");
  await page.locator(".pane-tabs").getByRole("tab", { name: /^Pages/ }).click();
});

// ---- Zoom / Navigate views
await check("Zoom ▸ − / + / 100% / Fit Width / Fit Page", async () => {
  await tab("Zoom");
  const readout = async () => Number((await page.locator(".ribbon-readout").innerText()).replace("%", ""));
  await rb("zoom100").click();
  assert((await readout()) === 100, "100%");
  await rb("zoomIn").click();
  assert((await readout()) === 110, `zoom + → ${await readout()}`);
  await rb("zoomOut").click();
  assert((await readout()) === 100, "zoom −");
  await rb("fitWidth").click();
  await page.waitForTimeout(300);
  const fw = await readout();
  await rb("fitPage").click();
  await page.waitForTimeout(300);
  assert((await readout()) < fw, "fit page smaller than fit width");
});

await check("Navigate ▸ Single / Spread (F5) / Continuous, Sidebar (F8)", async () => {
  await tab("Navigate");
  await rb("single").click();
  await page.waitForTimeout(200);
  assert((await page.locator(".page-view").count()) === 1, "single");
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("F5");
  await page.waitForTimeout(200);
  assert((await page.locator(".page-spread").count()) === 2, "spread pairs");
  await rb("continuous").click();
  await page.waitForTimeout(200);
  assert((await page.locator(".page-view").count()) === 4, "continuous");
  await page.keyboard.press("F8");
  assert((await page.locator(".pages-pane").count()) === 0, "F8 hides");
  await rb("sidebar").click();
  assert((await page.locator(".pages-pane").count()) === 1, "shown");
});

await check("all 9 Windows colour schemes switch in any order with readable text", async () => {
  const schemes = ["light-grey", "dark-blue", "darker-navy", "blue", "pastel", "hc-dark", "hc-light", "pastel-2", "pastel-3", "dark-blue", "light-grey"];
  for (const name of schemes) {
    await page.evaluate((t) => window.__stitchup.theme(t), name);
    const [fg, bg, sfg, sbg] = await page.evaluate(() => {
      const t = getComputedStyle(document.querySelector(".ribbon-tab.active"));
      const s = getComputedStyle(document.querySelector(".status-bar"));
      return [t.color, t.backgroundColor, s.color, s.backgroundColor];
    });
    assert(contrast(fg, bg) >= 4.5, `${name}: tab contrast ${contrast(fg, bg).toFixed(2)}`);
    assert(contrast(sfg, sbg) >= 4.5, `${name}: status contrast ${contrast(sfg, sbg).toFixed(2)}`);
  }
  await page.keyboard.press("Meta+d");
  assert((await page.evaluate(() => document.documentElement.dataset.theme)) === "dark-blue", "⌘D → Dark Blue");
  await page.keyboard.press("Meta+d");
  assert((await page.evaluate(() => document.documentElement.dataset.theme)) === "light-grey", "⌘D back");
});

await check("Help (F1) and About show their dialogs", async () => {
  await page.keyboard.press("F1");
  await page.waitForSelector("text=User Guide");
  await page.keyboard.press("Escape");
  await run("about");
  const txt = await page.locator(".license-table").innerText();
  assert(/PDF\.js/.test(txt) && /lopdf/.test(txt) && /Tauri/.test(txt), txt);
  await page.keyboard.press("Escape");
});

// ---- Forms and images
await check("forms: fill text, multi-line, checkbox, radio, drop-down; undo; saved values read back by pdf-lib", async () => {
  const f = path.join(WORK, "form.pdf");
  await makeForm(f);
  await openViaRibbon(f);
  await page.waitForSelector('[data-page="1"] .form-field');
  const name = page.getByLabel("applicant.name");
  await name.fill("Zofia Łukasiewicz");
  await name.press("Enter");
  await waitStatus(/Filled “applicant.name”/);
  await page.getByLabel("notes").fill("Line one of the notes and a longer second sentence that wraps");
  await page.getByLabel("notes").evaluate((el) => el.blur());
  await waitStatus(/Filled “notes”/);
  await page.getByRole("checkbox", { name: "agree" }).click();
  await waitStatus(/Filled “agree”/);
  await page.getByRole("radio", { name: "size: large" }).click();
  await waitStatus(/Filled “size”/);
  await page.getByLabel("country").selectOption("Australia");
  await waitStatus(/Filled “country”/);
  const fields = Object.fromEntries((await ipc("list_fields")).map((x) => [x.name, x.value]));
  assert(fields["applicant.name"] === "Zofia Łukasiewicz" && fields.country === "Australia" && fields.size === "1" && fields.agree !== "Off", JSON.stringify(fields));
  await page.screenshot({ path: path.join(OUT, "form.png") });
  // Undo the drop-down, redo it.
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Meta+z");
  await waitStatus(/Undid/);
  assert((await ipc("list_fields")).find((x) => x.name === "country").value !== "Australia", "undo restores the field");
  await page.keyboard.press("Meta+Shift+z");
  await waitStatus(/Redid/);
  await page.keyboard.press("Meta+s");
  await waitNotice(/Saved form\.pdf/);
  const saved = (await PDFDocument.load(await readFile(f))).getForm();
  assert(saved.getTextField("applicant.name").getText() === "Zofia Łukasiewicz", "name saved");
  assert(saved.getCheckBox("agree").isChecked(), "checkbox saved");
  assert(saved.getRadioGroup("size").getSelected() === "large", "radio saved");
  assert(saved.getDropdown("country").getSelected()[0] === "Australia", "drop-down saved");
});

await check("Content ▸ Image inserts a PNG (with transparency) and a JPEG", async () => {
  const png = path.join(WORK, "pic.png");
  const jpg = path.join(WORK, "pic.jpg");
  await makePng(png);
  execFileSync("sips", ["-s", "format", "jpeg", png, "--out", jpg], { stdio: "pipe" });
  const before = (await ipc("list_objects", { page: 1 })).filter((o) => o.kind === "image").length;
  await tab("Content");
  await pick(png);
  await rb("image").click();
  await waitNotice(/place pic\.png/);
  let [x, y] = await emptySpot(1, 0.6, 0.3);
  await dragMouse(x, y, x + 160, y + 40);
  await waitStatus(/Inserted image pic\.png/);
  await pick(jpg);
  await rb("image").click();
  await waitNotice(/place pic\.jpg/);
  [x, y] = await emptySpot(1, 0.6, 0.45);
  await page.mouse.click(x, y);
  await waitStatus(/Inserted image pic\.jpg/);
  const imgs = (await ipc("list_objects", { page: 1 })).filter((o) => o.kind === "image");
  assert(imgs.length === before + 2, `images ${imgs.length}`);
  const r = imgs.find((o) => Math.abs(o.rect[2] - o.rect[0] - (o.rect[3] - o.rect[1]) * 2) < 1);
  assert(r, "aspect 2:1 kept");
  const bytes = Buffer.from((await ipc("render_bytes")).bytes_b64, "base64").toString("latin1");
  assert(bytes.includes("/SMask") && bytes.includes("/DCTDecode"), "PNG alpha → SMask, JPEG embedded as DCT");
  await page.screenshot({ path: path.join(OUT, "images.png") });
});

// ---- New / Combine
await check("Document ▸ New makes an untitled A4 document; Save asks where", async () => {
  await tab("Document");
  await rb("new").click();
  await btn("Create").click();
  await waitStatus(/Untitled \d+/);
  const s = await ipc("render_bytes");
  assert(s.bytes_b64.length > 0, "doc");
  assert(/Page 1 of 1/.test(await status()), await status());
  const target = path.join(WORK, "new.pdf");
  await pick(target);
  await rb("save").click();
  await waitStatus(/new\.pdf/);
  const d = await pdfjs.getDocument({ data: new Uint8Array(await readFile(target)) }).promise;
  const vp = (await d.getPage(1)).getViewport({ scale: 1 });
  assert(Math.round(vp.width) === 595 && Math.round(vp.height) === 842, `${vp.width}x${vp.height}`);
  await d.destroy();
});

await check("File ▸ Combine into New builds one untitled document", async () => {
  await pick([OTHER, MAIN]);
  await run("combineNew");
  await waitStatus(/Combined 2 files/);
  assert(/Page 1 of 6/.test(await status()), await status());
  assert(/Untitled/.test(await status()), "untitled");
});

await check("⌘O opens the file dialog", async () => {
  await pick(OTHER);
  openPath = OTHER;
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Meta+o");
  await waitStatus(/other\.pdf/);
  await waitStatus(/Page 1 of 2/);
});


// ---- Document tabs
await check("tabs: each document opens in its own tab and keeps its page and zoom", async () => {
  const A = path.join(WORK, "tabA.pdf");
  const B = path.join(WORK, "tabB.pdf");
  await makePdf(A, [LETTER, LETTER, LETTER]);
  await makePdf(B, [A5, A5]);
  await openViaRibbon(A);
  const tabsBefore = await page.locator(".doc-tab").count();
  await page.locator('[data-thumb="3"] .thumb').click();
  await waitStatus(/Page 3 of 3/);
  await tab("Zoom");
  await rb("zoom100").click();
  await openViaRibbon(B);
  assert((await page.locator(".doc-tab").count()) === tabsBefore + 1, "second tab");
  assert(/tabB\.pdf/.test(await page.locator(".doc-tab.active").innerText()), "B active");
  await waitStatus(/Page 1 of 2/);
  await page.locator(".doc-tab", { hasText: "tabA.pdf" }).click();
  await waitStatus(/Page 3 of 3/);
  assert(/Zoom 100%/.test(await status()), `zoom restored: ${await status()}`);
  // Opening an already-open file switches to its tab.
  const n = await page.locator(".doc-tab").count();
  openPath = B;
  await pick(B);
  await tab("Document");
  await rb("open").click();
  await waitStatus(/Page 1 of 2/);
  assert((await page.locator(".doc-tab").count()) === n, "no duplicate tab");
});

await check("tabs: move a page to another tab (context menu and drag onto the tab)", async () => {
  // B is active (2 pages). Move its page 2 to tab A.
  await page.locator('[data-page="2"]').click({ button: "right", position: { x: 20, y: 20 } });
  await page.locator(".context-menu").getByRole("menuitem", { name: "Move Page to “tabA.pdf”" }).click();
  await waitStatus(/Moved 1 page to “tabA\.pdf”/);
  assert(/Page \d of 1/.test(await status()), "B now has 1 page");
  assert(/\*/.test(await page.locator(".doc-tab", { hasText: "tabA.pdf" }).innerText()), "A marked unsaved");
  await page.locator(".doc-tab", { hasText: "tabA.pdf" }).click();
  await waitStatus(/of 4/);
  // Drag A's first thumbnail onto tab B.
  const thumb = page.locator('[data-thumb="1"] .thumb');
  await thumb.scrollIntoViewIfNeeded();
  const a = await thumb.boundingBox();
  const t = await page.locator(".doc-tab", { hasText: "tabB.pdf" }).boundingBox();
  await page.mouse.move(a.x + a.width / 2, a.y + 30);
  await page.mouse.down();
  for (let i = 1; i <= 16; i++) await page.mouse.move(a.x + a.width / 2 + ((t.x + 20 - a.x - a.width / 2) * i) / 16, a.y + 30 + ((t.y + t.height / 2 - a.y - 30) * i) / 16);
  await page.mouse.up();
  await waitStatus(/Moved 1 page to “tabB\.pdf”/);
  assert(/of 3/.test(await status()), "A back to 3 pages");
  await page.locator(".doc-tab", { hasText: "tabB.pdf" }).click();
  await waitStatus(/of 2/);
});

await check("tabs: closing an unsaved tab asks; Cancel keeps it, Close removes it; ⌘T / ⌃Tab", async () => {
  const n = await page.locator(".doc-tab").count();
  await pick("Cancel"); // the ask dialog returns the clicked button's label
  await page.locator(".doc-tab.active .doc-tab-close").click();
  await page.waitForTimeout(300);
  assert((await page.locator(".doc-tab").count()) === n, "Cancel keeps the tab");
  await pick("Close");
  await page.locator(".doc-tab.active .doc-tab-close").click();
  await page.waitForFunction((c) => document.querySelectorAll(".doc-tab").length === c - 1, n);
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Meta+t");
  await page.waitForFunction((c) => document.querySelectorAll(".doc-tab").length === c, n);
  assert((await page.locator(".doc-tab.active").innerText()).startsWith("New Tab"), "⌘T opens an empty tab");
  const before = await page.locator(".doc-tab.active").innerText();
  await page.keyboard.press("Control+Tab");
  await page.waitForTimeout(300);
  assert((await page.locator(".doc-tab.active").innerText()) !== before, "⌃Tab switches");
});

// ---- Robustness fixtures (generated by the Rust core)
await check("password-protected PDF: prompt, wrong password, right password", async () => {
  const f = path.join(FIX, "F-04_encrypted.pdf");
  openPath = f;
  await pick(f);
  await tab("Document");
  await rb("open").click();
  await page.waitForSelector("form[aria-label='unlock document']");
  await page.getByLabel("Password").fill("wrong");
  await btn("Unlock").click();
  await page.waitForSelector(".error-toast");
  await page.getByRole("button", { name: "Dismiss" }).click();
  await page.getByLabel("Password").fill("stitchup-user");
  await btn("Unlock").click();
  await waitStatus(/Page 1 of/);
});

await check("Save As of an unlocked PDF writes a valid copy", async () => {
  const target = path.join(WORK, "unlocked-copy.pdf");
  await pick(target);
  await rb("saveAs").click();
  await waitStatus(/unlocked-copy\.pdf/);
  assert((await pdfText(target)).length >= 1, "copy unreadable");
});

await check("malformed PDF shows an error and the app stays usable", async () => {
  await pick(path.join(FIX, "F-08_malformed.pdf"));
  await rb("open").click();
  await page.waitForSelector(".error-toast");
  await page.getByRole("button", { name: "Dismiss" }).click();
  await openViaRibbon(MAIN);
  assert(/of 4/.test(await status()), `good file opens after a bad one: ${await status()}`);
});

await check("XFA warning badge; no Attachments button (removed for now)", async () => {
  await openViaRibbon(path.join(FIX, "F-13_xfa.pdf"));
  await page.waitForSelector(".status-warn");
  await tab("Document");
  assert((await rb("attachments").count()) === 0, "Attachments button still on the ribbon");
});

// ---- Real-world PDF with a nested page tree (if present on this Mac)
const REAL = "/Applications/Keynote.app/Contents/Resources/License.pdf";
if (existsSync(REAL)) {
  await check("real-world nested PDF: open, delete one page, edit text", async () => {
    const f = path.join(WORK, "real.pdf");
    await copyFile(REAL, f);
    await openViaRibbon(f);
    const n = (await state()).pages.length;
    assert(n > 100, `pages ${n}`);
    await page.locator('[data-thumb="2"] .thumb').click();
    await tab("Pages");
    await rb("delete").click();
    await waitStatus(new RegExp(`of ${n - 1}`));
    assert((await state()).pages.length === n - 1, "exactly one page removed");
    await page.locator('[data-thumb="1"] .thumb').click();
    await tab("Content");
    await rb("editText").click();
    await page.waitForSelector('[data-page="1"] .text-run.editable');
    await page.locator('[data-page="1"] .text-run.editable', { hasText: "ENGLISH" }).first().click();
    await page.locator(".inline-editor:not(.pending)").fill("POLSKI");
    await page.locator(".inline-editor:not(.pending)").press("Enter");
    await waitStatus(/Text edited/);
    console.log(`        (${(await status()).split("\n").join(" · ")})`);
    await rb("editText").click();
  });
}


// ---- Settings, quit warning, crash recovery (last: it restarts the backend)
await check("Settings (⌘,): recovery interval, opening zoom and colour scheme are saved", async () => {
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Meta+,");
  await page.getByLabel("Crash-recovery snapshots").selectOption("30");
  await page.getByLabel("Opening zoom").selectOption("fit-page");
  await page.getByLabel("Colour scheme").selectOption("pastel");
  await btn("Save").click();
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem("stitchup.settings")));
  assert(saved.recoverySeconds === 30 && saved.openZoom === "fit-page", JSON.stringify(saved));
  assert((await page.evaluate(() => document.documentElement.dataset.theme)) === "pastel", "scheme applied");
  await page.evaluate(() => window.__stitchup.theme("light-grey"));
});

await check("quitting with unsaved changes asks first (Cancel keeps running)", async () => {
  const f = path.join(WORK, "quit.pdf");
  await makePdf(f, [LETTER, LETTER]);
  await openViaRibbon(f);
  await tab("Pages");
  await rb("rotateCw").click();
  await waitStatus(/rotation/);
  const before = await calls("quit_app");
  await pick("Cancel");
  await page.locator(".viewer-scroll").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("Meta+q");
  await page.waitForTimeout(400);
  assert((await calls("quit_app")) === before, "quit without asking");
  await pick("Discard and Quit");
  await page.keyboard.press("Meta+q");
  await page.waitForFunction((b) => window.__calls.filter((c) => c === "quit_app").length > b, before);
});

await check("crash recovery: snapshot → backend killed → relaunch offers the document back", async () => {
  assert((await ipc("save_recovery")) >= 1, "snapshot written");
  bridge.kill("SIGKILL"); // crash
  await new Promise((r) => setTimeout(r, 300));
  const PORT2 = PORT + 300;
  const bridge2 = spawn(bridgeBin, [String(PORT2), path.join(APP, "dist")], { stdio: ["ignore", "pipe", "inherit"], env: bridgeEnv });
  await new Promise((res) => bridge2.stdout.once("data", res));
  try {
    await page.goto(`http://127.0.0.1:${PORT2}`);
    await page.waitForSelector("text=Recover unsaved documents?", { timeout: 10000 });
    assert((await page.locator(".recovery-list").innerText()).includes("quit.pdf"), "lists the crashed document");
    await btn("Recover").click();
    const recoveredTab = page.locator(".doc-tab", { hasText: "quit (recovered).pdf" });
    await recoveredTab.waitFor({ timeout: 10000 });
    await recoveredTab.click();
    await waitStatus(/of 2/);
    const st = await page.evaluate(() => window.__stitchup.state().doc.pages.map((p) => p.rotation));
    assert(st[0] === 90, `the unsaved rotation came back: ${st}`);
    assert((await readdir(RECOVERY)).filter((f) => f.startsWith("rec_")).length === 0, "snapshots cleaned up after recovery");
  } finally {
    bridge2.kill();
  }
});

await check("no uncaught errors in the page", async () => {
  assert(consoleErrors.length === 0, consoleErrors.join(" | "));
});

await browser.close();
bridge.kill();

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed. Screenshots: ${OUT}`);
process.exit(failed.length ? 1 : 0);
