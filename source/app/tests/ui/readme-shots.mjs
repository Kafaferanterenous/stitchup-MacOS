//! Screenshots for the README (docs/screenshots/*.png): the REAL UI + REAL
//! backend (dev bridge), driven like the e2e test, on a demo document with
//! original text.
//!
//!   npm run build && node tests/ui/readme-shots.mjs <out-dir>

import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { PDFDocument, StandardFonts, rgb } from "pdf-lib";

const APP = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const OUT = path.resolve(process.argv[2] ?? path.join(APP, "..", "..", "docs", "screenshots"));
const WORK = path.join(os.tmpdir(), "stitchup-shots");
const PORT = 1800 + Math.floor(Math.random() * 100);

async function demo(file) {
  const doc = await PDFDocument.create();
  const sans = await doc.embedFont(StandardFonts.Helvetica);
  const bold = await doc.embedFont(StandardFonts.HelveticaBold);
  const serif = await doc.embedFont(StandardFonts.TimesRoman);
  doc.setTitle("Stitchup demo");
  const para = [
    "Stitchup is an offline PDF editor. Everything happens on this computer:",
    "nothing is uploaded, and the original file is only replaced when you save.",
    "Double-click any line to change its words in the document's own font.",
    "Highlight, underline, notes, stamps, shapes and links are real PDF",
    "annotations, so they look the same in Preview and Acrobat.",
  ];
  for (let n = 1; n <= 3; n++) {
    const p = doc.addPage([612, 792]);
    p.drawRectangle({ x: 0, y: 742, width: 612, height: 50, color: rgb(0.18, 0.36, 0.65) });
    p.drawText(`Quarterly report — section ${n}`, { x: 54, y: 760, size: 18, font: bold, color: rgb(1, 1, 1) });
    p.drawText("Summary", { x: 54, y: 690, size: 22, font: bold });
    para.forEach((l, i) => p.drawText(l, { x: 54, y: 655 - i * 20, size: 12, font: serif }));
    p.drawText("Results", { x: 54, y: 520, size: 16, font: bold });
    const rows = [["Region", "Q1", "Q2", "Q3"], ["North", "1 204", "1 380", "1 512"], ["South", "980", "1 044", "1 130"], ["West", "412", "466", "530"]];
    rows.forEach((r, i) => r.forEach((c, j) => p.drawText(c, { x: 54 + j * 120, y: 490 - i * 22, size: 11, font: i === 0 ? bold : sans })));
    p.drawLine({ start: { x: 54, y: 482 }, end: { x: 520, y: 482 }, thickness: 0.8, color: rgb(0.5, 0.5, 0.5) });
    p.drawText(`Page ${n} of 3`, { x: 280, y: 40, size: 9, font: sans, color: rgb(0.4, 0.4, 0.4) });
  }
  const form = doc.getForm();
  const p1 = doc.getPage(0);
  p1.drawText("Reviewer", { x: 54, y: 340, size: 11, font: sans });
  form.createTextField("reviewer").addToPage(p1, { x: 140, y: 332, width: 220, height: 22 });
  p1.drawText("Approved", { x: 54, y: 300, size: 11, font: sans });
  form.createCheckBox("approved").addToPage(p1, { x: 140, y: 296, width: 16, height: 16 });
  await writeFile(file, await doc.save({ useObjectStreams: false }));
}

await mkdir(OUT, { recursive: true });
await mkdir(WORK, { recursive: true });
const file = path.join(WORK, "demo.pdf");
await demo(file);

const bridge = spawn(path.join(APP, "src-tauri/target/release/examples/dev_bridge"), [String(PORT), path.join(APP, "dist")]);
await new Promise((r) => bridge.stdout.once("data", r));
const ipc = async (cmd, args = {}) => {
  const j = await (await fetch(`http://127.0.0.1:${PORT}/ipc/${cmd}`, { method: "POST", body: JSON.stringify(args) })).json();
  if ("err" in j) throw new Error(j.err.message);
  return j.ok;
};
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1360, height: 860 }, deviceScaleFactor: 2 });
await page.addInitScript(() => {
  window.__dialog = [];
  window.__TAURI_INTERNALS__ = {
    async invoke(cmd, args) {
      if (cmd.startsWith("plugin:dialog|")) return window.__dialog.shift() ?? null;
      const j = await (await fetch(`/ipc/${cmd}`, { method: "POST", body: JSON.stringify(args ?? {}) })).json();
      if ("err" in j) throw j.err;
      return j.ok;
    },
    transformCallback: () => 0,
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
  };
});
await page.goto(`http://127.0.0.1:${PORT}`);
const run = (id) => page.evaluate((i) => window.__stitchup.run(i), id);
const theme = (t) => page.evaluate((x) => window.__stitchup.theme(x), t);
const tab = (n) => page.locator(".ribbon-tabs").getByRole("tab", { name: n, exact: true }).click();
const settle = () => page.waitForTimeout(900);
const shot = (name) => page.screenshot({ path: path.join(OUT, name) });

// 1. Empty window (Pastel, like the Windows screenshot)
await theme("pastel");
await shot("empty-pastel.png");

// 2. Document with annotations (Light Grey, Annotate tab)
await theme("light-grey");
await page.evaluate((p) => window.__dialog.push(p), file);
await run("open");
await page.waitForSelector('[data-page="1"] canvas');
// Through the app's own store, so the page re-renders like a real edit.
const annotate = (spec) => page.evaluate((sp) => window.__stitchup.store.annotate(1, sp), spec);
await annotate({ kind: "highlight", boxes: [[54, 650, 420, 668]], color: [1, 0.92, 0.23] });
await annotate({ kind: "note", at: [470, 700], text: "Check the Q3 figures", color: [1, 0.85, 0.2] });
await annotate({ kind: "square", rect: [48, 400, 530, 512], color: [0.82, 0.01, 0.11], fill: null, width: 1.5 });
await annotate({ kind: "stamp", rect: [400, 230, 560, 280], label: "Approved", color: [0.1, 0.55, 0.2] });
await annotate({ kind: "underline", boxes: [[54, 571, 360, 588]], color: [0.82, 0.01, 0.11] });
await run("fitWidth");
await settle();
await tab("Annotate");
await settle();
await shot("annotate-light-grey.png");

// 3. Inline text editing (Blue)
await theme("blue");
await tab("Content");
const line = page.locator('[data-page="1"] .text-run', { hasText: "Double-click any line" }).first();
await line.dblclick();
await page.locator(".inline-editor").fill("Double-click any line to rewrite it — it stays in the original font.");
await settle();
await shot("edit-text-blue.png");
await page.locator(".inline-editor").press("Enter");
await settle();

// 4. Forms + Content tab (Dark Blue)
await theme("dark-blue");
await page.getByLabel("reviewer").fill("Zofia Łukasiewicz");
await page.getByLabel("reviewer").press("Enter");
await page.getByRole("checkbox", { name: "approved" }).click();
await settle();
await page.locator('[data-page="1"]').evaluate((el) => el.scrollIntoView({ block: "center" }));
await settle();
await shot("forms-dark-blue.png");

// 5. Print dialog (Pastel 3)
await theme("pastel-3");
await tab("Document");
await run("print");
await page.waitForTimeout(1500);
await shot("print-pastel-3.png");
await page.keyboard.press("Escape");

await browser.close();
bridge.kill();
console.log(`screenshots → ${OUT}`);
