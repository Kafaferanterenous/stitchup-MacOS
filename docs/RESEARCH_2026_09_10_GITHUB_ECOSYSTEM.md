# Stitchup — Ecosystem Research: Open-Source PDF Editors and Building Blocks

Date: 2026-09-10 (Australia/Adelaide)
Status: research-only. Web sources checked; NO source code, package, binary,
font, or model was downloaded, installed, built, or executed. This document
extends `spec/RESEARCH_2026_08_20_PDF_EDITOR_ECOSYSTEM.md`.

## Purpose

Answer three questions with evidence:

1. What are the best fully featured open-source PDF editors, and what can we
   learn from them without copying them?
2. Which GitHub projects demonstrate the architecture we plan (Track A: Tauri +
   PDF.js/pdf-lib; Track B: native)?
3. Which permissive-license building blocks can Stitchup actually use, and what
   are their exact licenses, versions, and caveats?

Clean-room rule stands: we study public behavior and architecture, then write
independent code. We do not copy any of these projects' code, assets, or
distinctive UI.

---

## 1. Fully featured open-source PDF editors (learn, do not copy)

| Project | Repo | Stack | License | Notable / what to learn |
|---|---|---|---|---|
| Stirling PDF | `Stirling-Tools/Stirling-PDF` | Java/Spring, React SPA, **Tauri desktop client**, Python sidecar | open-core: MIT core + `proprietary/` (SAML SSO, audit, teams) | ~91k stars. Feature breadth (50+ tools). License-compliance pipeline (`allowed-licenses.json`, Gradle license-report). Docker regressions are a caution about magnitude of ops, not an endorsement of heavy architecture. |
| PDFsam Basic | `torakiki/pdfsam` | Java | AGPL-3.0 | Page ops only (merge/split/rotate). Honest scope discipline. AGPL = study-only for us. |
| PDF Arranger | `pdfarranger/pdfarranger` | Python/GTK | GPL-3.0 | Visual page grid: drag reorder, rotate, crop. Thumbnail interaction patterns. GPL = study-only. |
| Okular | KDE/okular | C++/Qt | GPL-2.0+ | Viewer + annotation + signature validation. Not a content editor. |
| ONLYOFFICE Docs | ONLYOFFICE | C++/JS | AGPL-3.0 | True in-place text edit + forms + collab. AGPL = incompatible with permissive product (study-only). |
| LibreOffice Draw | LibreOffice/core | C++ | MPL-2.0 | Imports PDF as editable objects; layout fidelity is the known pain point. Confirms our "overlay, not reflow" stance. |
| Xournal++ | `xournalpp/xournalpp` | C++ | GPL-2.0 | PDF-as-background annotation layers, pen input. GPL = study-only. |
| Inkscape | `inkscape/inkscape` | C++ | GPL-2.0+ | One-page-at-a-time vector edit via Poppler. GPL = study-only. |

**Reading:** no single open-source app fully replaces Acrobat; the category
splits into viewers/annotators, page-grid tools, and content-import editors.
Stitchup's niche is the *portable desktop editor* combining all three with
permissive licensing.

---

## 2. GitHub projects with our exact architecture (highest value)

| Project | Stack | License | Why it matters |
|---|---|---|---|
| **OpenAEC-Foundation/open-pdf-studio** (719★, v1.47.11, active) | Tauri 2 + SolidJS + **PDFium worker pool** (progressive tiling) + PDF.js text layer + pdf-lib | LGPL-3.0 | Closest public analogue to our Track A. Learn: multi-process PDFium for render isolation, ribbon layout, PDF/A detection. LGPL → study architecture only; do not copy modules into a closed product. |
| **OpenAEC-Foundation/open-pdf-studio-v3** | Rust + **Slint** + `pdfium-render` (native UI, no browser) | LGPL-3.0 (to be verified) | Track B reference: native UI + PDFium on a worker thread via Command/Event API. |
| **jasonulbright/Open-PDF-Studio** (12★) | Tauri v2 + React + pdf.js + pdf-lib + **pikepdf/qpdf + pdfminer.six + pyHanko + Ghostscript** (embedded Python 3.14 sidecar) | MIT | Nearly our proposed stack + Python sidecar. Best "feature → building block" table found (see §4). **Avoid its Ghostscript (AGPL-3.0) dependency**; treat all AGPL parts as out-of-scope. |
| **wbp318/HermosaPDF** | Tauri v2 + React + pdfjs-dist + pdf-lib + **lopdf (Rust)** + Konva + tesseract.js; **retro Win98 skin** | (check LICENSE before use) | Validates our pattern: Rust `lopdf` for decryption/structural normalization of "in-the-wild" PDFs, Konva overlay, pdf.js render. Retro theme aesthetic is a useful UX reference for the XP look. Its Claude-AI feature conflicts with our privacy goals — we keep AI as an optional local-only provider. |
| **kudosscience/pdf-chisel** (MIT) | Electron + **PDFium via native N-API addon** | MIT | Proof that PDFium's edit API does real in-place text/image edits — the technique for a later "edit original text" stage. |
| **CasualOffice/pdf** (docs only) | Design docs: **PDFium everywhere** (WASM `@embedpdf/pdfium` + native `pdfium-render`), Tauri desktop | Apache-2.0 | Architecture/RESEARCH writeup with a permissive-only license table; recommend reading `docs/RESEARCH.md` + `docs/ARCHITECTURE.md`. |
| **zParik/Qyra** | Tauri 2 + React + **MuPDF** Rust FFI | GPL-3.0 | Shows MuPDF's capability — and why it stays out (AGPL/GPL). Do not use. |
| Browser-based overlay editors: **akkikumar72/Akki-Pdf-editor**, **KurutoDenzeru/Inkoro**, **Maksimkoho/editorapdf**, **antonprafanto/pdfkit**, **pakcli/offline-pdf-editor** | React + pdf.js + pdf-lib + Fabric.js/Konva + dnd-kit | MIT (verify each) | Common textbook pattern: render with pdf.js, model edits as **overlay layers**, compile with pdf-lib on export. Matches our overlay-layer design (STAGE0 docs). Good for behavior study of coordinate normalization + export. |

---

## 3. Permissive building blocks (the ones we can actually use)

### 3.1 Renderers / text layer

| Component | Version/status | License | Notes |
|---|---|---|---|
| PDF.js (`pdfjs-dist`) | current stable; Apache-2.0 | Apache-2.0 | Renderer + text layer + search. Docs are CC BY-SA 2.5; dist carries third-party notices. Existing research covers it. |
| **@embedpdf/pdfium** (WASM PDFium) | 2.15.0, 2026-08-04, ~382k weekly dl | MIT (wrapper) + PDFium Apache-2.0 | Native-quality rendering in JS/WASM; browser + worker. Get this only if we want a PDFium painless WASM path later. Repo `embedpdf/embed-pdf-viewer` is MIT; **exclude `cloudpdf/server`** (Fair Core License — not free to run). |
| **pdfium-render** (Rust) | 0.9.3, `pdfium_7881` | MIT OR Apache-2.0 | Idiomatic Rust binding; needs a PDFium binary (see below). Native Track B or Rust backend candidate. Thread-safe via internal mutex (PDFium is single-threaded). |
| PDFium | up to chromium/7988 | **BSD-3-Clause AND Apache-2.0** (dual, since 2022) | Big dependency surface: bundles FreeType (FTL), ICU (Unicode v3), libjpeg-turbo, OpenJPEG, libpng, libtiff, zlib, Abseil etc. Builds: `bblanchon/pdfium-binaries` (MIT build scripts; WASM builds use non-growable heap — use `paulocoutinhox/pdfium-lib` for growable WASM). Needs a dedicated review before adoption (as our handoff said). |

### 3.2 Page operations / structural / forms

| Component | Version/status | License | Notes |
|---|---|---|---|
| **pdf-lib** | 1.4.0 | MIT (+ `@pdf-lib/fontkit` for font embedding) | JS page ops, form create/fill/flatten, metadata, overlays. No redaction, no real signing, limited encryption. Core Stage 1/2 candidate. |
| **lopdf** (Rust) | 0.44.0 (2026-07-10), 2237★, active | MIT (except bundled Montserrat font) | Object streams, decryption incl. revision 5/6, encrypt, font embedding, `load_with_password`. **Pin ≥ 0.42.0** — RUSTSEC-2026-0187 (stack overflow on deeply nested objects) fixed in 0.42.0. Rust structural normalizer for Tauri commands. |
| QPDF | stable | Apache-2.0 | Structural transforms, linearize, encrypt. WASM packaging: `jsscheller/qpdf-wasm` (Apache-2.0) or `neslinesli93/qpdf-wasm` (ISC, 0.3.0, 2025-06). Native C++ binary if we accept a sidecar. |
| **pikepdf** (Python, wraps QPDF) | 2.7k★, active | **MPL-2.0** (file-level copyleft; compatible with closed source) | Page ops + **redaction API** + sanitizer (`remove_javascript()` etc.) + linearize + encryption. MPL lets us use it, but it drags Python + QPDF; keep behind adapter if chosen. |

### 3.3 Overlay / annotation / ink

| Component | License | Used by | Notes |
|---|---|---|---|
| **Konva + react-konva** | MIT | HermosaPDF, Open-Satchel | Canvas overlay, hit-testing, transform handles. |
| **Fabric.js** | MIT | openpdf patches, many editors | Canvas object model. |
| **perfect-freehand** | MIT | Qyra | Smooth stylus/freehand ink. |
| **dnd-kit** | MIT | Inkoro, offline-pdf-editor | Page thumbnail drag/reorder. |
| **react-moveable** | MIT | Inkoro | Selection/resize/rotate boxes. |

### 3.4 Signing (Stage 3 gate)

| Component | License | Notes |
|---|---|---|
| **@signpdf** family (`@signpdf/signpdf`, `placeholder-pdf-lib`, `signer-p12`) | MIT (peer dep **node-forge**, BSD-3) | PAdES / PKCS#7 detached signing; needs a signature placeholder (pdf-lib helper exists). |
| **pyHanko** | MIT | Python PAdES signing/validation; heavier, MPL-compatible via Python sidecar only if adopted. |

### 3.5 OCR (Stage 4 gate — language data must be recorded)

| Component | License | Notes |
|---|---|---|
| Tesseract | Apache-2.0 | Engine; tessdata/langdata licensing recorded separately. |
| **tesseract.js** | Apache-2.0 | Runs in WebView; heavier at startup. |
| OCRmyPDF | MPL-2.0 | Excellent reference for OCR pipeline, but **depends on Ghostscript (AGPL)** → keep out or document the gate. |

### 3.6 Text extraction / metrics (needed later for limited text editing)

| Component | License | Notes |
|---|---|---|
| **pdfminer.six** | MIT | Text runs + font metrics/encodings — the proven basis for close-match font substitution on overlay text edit. Requires Python sidecar. |
| PDF.js `getTextContent()` | Apache-2.0 | JS-only alternative for selection/search; less font-metric fidelity for edits. |

---

## 4. License-compatibility matrix for Stitchup's distribution goals

Stitchup targets a portable, offline-first product with an "About → Licenses"
section. Recommended policy:

- **Admissible in default build:** MIT, ISC, BSD-3/2, Apache-2.0, MPL-2.0,
  Unicode license v3, zlib/libpng.
- **Study-only (copyleft, no code reuse):** AGPL-3.0 (PDFsam, ONLYOFFICE,
  Open Satchel), GPL-2/3 (Xournal++, Okular, PDF Arranger, Inkscape, Qyra),
  LGPL-3.0 (Open PDF Studio — study architecture only).
- **Gate behind separate review/feature flag:** AGPL components (MuPDF,
  Ghostscript), PDFium (large notice surface), OCR language data, any Open-Core
  code (Stirling-PDF `proprietary/`), Fair-Core (EmbedPDF `cloudpdf/server`).
- **Never:** copying UI assets/logos/measurements from Microsoft, PDF Gear,
  Nitro Pro, or the XP.css/98.css projects' distinctive artwork. Theme is
  "XP-inspired, independently drawn," with dark/blue variants and WCAG
  contrast (buttons/menus must be readable) — a Stage 1 milestone.

Table for the record (verify hashes/notices at download time, per ledger):

| Component | License | Compatible with permissive distribution |
|---|---|---|
| PDF.js | Apache-2.0 | Yes |
| pdf-lib (+ fontkit) | MIT | Yes |
| lopdf | MIT | Yes |
| QPDF / qpdf-wasm | Apache-2.0 / ISC | Yes |
| pikepdf | MPL-2.0 | Yes (file-level source availability) |
| pdfminer.six | MIT | Yes |
| @signpdf / node-forge | MIT / BSD-3 | Yes |
| tesseract.js | Apache-2.0 | Yes (data separately gated) |
| @embedpdf/pdfium | MIT + Apache-2.0 (PDFium) | Yes (excl. cloudpdf/server) |
| pdfium-render | MIT OR Apache-2.0 | Yes |
| PDFium + bundled libs | BSD-3 + Apache-2.0 + FTL + ICU + zlib… | Yes, but big notice surface |
| MuPDF | AGPL / commercial | No (unless deliberate choice) |
| Ghostscript | AGPL-3.0 | No in default build |
| Stirling-PDF proprietary/ | commercial open-core | No |

---

## 5. Recommendation for Stitchup (baseline, subject to the size/quality experiment)

Stage 1-2 baseline (Track A, fastest path):

- Shell: **Tauri 2** (Rust backend), React + TypeScript + Vite frontend.
- Renderer + text layer: **PDF.js** (Apache-2.0). Parallel measured experiment
  vs **@embedpdf/pdfium** (WASM) to decide final renderer; both are admissible.
- Page ops/forms/overlay compile: **pdf-lib** (MIT) in the frontend.
- Rust structural normalizer: **lopdf** (MIT, ≥0.42) as Tauri commands for
  decryption/malformed-XRef handling and later native ops.
- Overlay canvas: **Konva / react-konva**, **perfect-freehand** for ink,
  **dnd-kit** for page reorder.
- Commit/export: own Tauri save path (temp + fsync + atomic rename) per
  STAGE0_ENGINE_INTERFACES.
- Optional provider slots (empty by default): OCR (tesseract.js, gated),
  signing (@signpdf, gated), redaction (pikepdf or lopdf-based custom, gated),
  AI (local endpoint only, gated).

Deferred decision notes:

- Keep QPDF/pikepdf and Python sidecar out unless the JS/Rust path cannot meet
  the Stage 1 acceptance contract (they add Python + C++ baggage).
- PDFium native (Track B) only if size/startup/fidelity experiment beats the
  Track A prototype; would need the dedicated notice review.
- Download **nothing** before the ledger (next doc) records version, source,
  expected size, hash, license, notices, and modification scope, and before it
  is approved.

---

## References (checked 2026-09-10)

- repoinsights/appselfhost reviews of Stirling-PDF (open-core module layout,
  license-report tooling)
- OpenAEC-Foundation/* (open-pdf-studio, open-pdf-studio-v3) READMEs + LICENSE
- jasonulbright/Open-PDF-Studio README (feature → component/license table)
- wbp318/HermosaPDF README; kudosscience/pdf-chisel README; zParik/Qyra README
- CasualOffice/pdf README; embedpdf/embed-pdf-viewer README + LICENSE;
  @embedpdf/pdfium npm page (2.15.0)
- crates.io: lopdf (0.44.0, MIT), pdfium-render (0.9.3, MIT OR Apache-2.0);
  GitHub: J-F-Liu/lopdf (RUSTSEC-2026-0187 advisory)
- npm: @neslinesli93/qpdf-wasm (0.3.0, ISC); GitHub jsscheller/qpdf-wasm
  (Apache-2.0)
- github.com/qpdf/qpdf; pikepdf README (MPL-2.0, redaction/sanitizer);
  pdf-lib.js.org (1.4.0)
- @signpdf/signpdf (3.3.0, MIT); node-signpdf README
- firecrawl/pdfium-rs THIRD-PARTY-NOTICES (PDFium component license table)
- OxygenPDF blog, Navanem, XDA, ctrlaltnod open-source editor surveys