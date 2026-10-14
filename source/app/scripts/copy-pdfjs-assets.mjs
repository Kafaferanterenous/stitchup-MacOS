// Copies pdf.js runtime resources that must be served alongside the app:
//   - standard_fonts/  (non-embedded base-14 fonts: Helvetica/Times/Courier...)
//   - cmaps/           (CID CMaps for CJK and predefined encodings)
// Without these, PDFs that rely on them render with missing/blank text.
// They are generated from node_modules into public/ (git-ignored), so run
// `npm install` before building. Wired via the prebuild/predev scripts.

import { cpSync, existsSync, mkdirSync, rmSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");
const src = resolve(root, "node_modules/pdfjs-dist");
const dest = resolve(root, "public");

const dirs = ["standard_fonts", "cmaps"];

if (!existsSync(src)) {
  console.error(`[pdfjs-assets] pdfjs-dist not found at ${src}. Run \`npm install\` first.`);
  process.exit(1);
}

mkdirSync(dest, { recursive: true });
for (const dir of dirs) {
  const from = resolve(src, dir);
  const to = resolve(dest, dir);
  if (!existsSync(from)) {
    console.error(`[pdfjs-assets] missing ${from}`);
    process.exit(1);
  }
  rmSync(to, { recursive: true, force: true });
  cpSync(from, to, { recursive: true });
  console.log(`[pdfjs-assets] ${dir} -> public/${dir}`);
}
