# Project 021 PDF editor ecosystem research

Date: 2026-08-20 (Australia/Adelaide)
Status: research-only. No PDF engines, repositories, fonts, OCR models,
packages, or binaries were downloaded, installed, built, or executed.

## Existing project state

The existing `prompt.txt` proposes Stitchup, a portable offline PDF editor with
a PDF Gear/Nitro-style workspace, rendering, page operations, annotations,
forms, signatures, encryption, redaction, Bates numbering, OCR, compression,
and optional local AI. It currently proposes Tauri/Electron, PDF.js, Rust or
native manipulation, and a license acknowledgements view.

The prompt is useful as a feature inventory, but true arbitrary text editing,
redaction, signatures, and PDF/A compliance need separate acceptance tests.
They should not be promised as simple overlay features.

## Candidate components and boundaries

### PDF.js, Apache-2.0

PDF.js is a strong rendering and text-layer reference. Its display layer exposes
rendering and document information, while the viewer layer demonstrates page
thumbnails, navigation, search, zoom, text selection, and annotation workflows.
It is Apache-2.0, but its documentation is CC BY-SA 2.5 and its distribution
contains third-party notices.

Use it as a viewer/UI reference or an optional frontend engine. It does not by
itself provide a complete native PDF editing engine. A browser-based PDF.js
frontend also means a JavaScript runtime and a WebView/system browser boundary.

### pdf-lib, MIT

pdf-lib is pure JavaScript and works in browsers and other JavaScript runtimes.
It can create and modify PDFs, add/remove/copy pages, draw text/images/vector
graphics, embed fonts, create/fill/flatten forms, attach files, and edit
metadata. It is a useful lightweight candidate for page operations, overlays,
forms, and generated documents.

It is not a complete replacement for a renderer, a high-fidelity arbitrary
text reflow engine, a robust encrypted-document editor, or a complete PDF
signature/redaction solution. Font embedding and Unicode behavior require
explicit tests.

### QPDF, Apache-2.0

QPDF is a C++ library and command-line tool for content-preserving PDF
transformations. It handles structural inspection, linearization, encryption,
page splitting/merging, and related low-level operations. It does not render
PDFs or provide a high-level page-content editor.

QPDF is a strong optional native backend for structural operations if its
complete dependency and packaging review passes. Keep it behind a narrow
interface and include its Apache license and notices.

### MuPDF, AGPL or commercial

MuPDF is fast, portable, and feature-rich for rendering, extraction,
conversion, redaction, signing, and more. Its open-source use is AGPL; a
commercial license is required when the application cannot comply with AGPL.
It is technically attractive but not compatible with a closed or permissively
licensed portable editor unless the project deliberately chooses AGPL or buys
a license.

### PDFium and bindings

PDFium is a powerful Chromium-derived renderer, but it is large and has a
complex dependency/build and third-party notice surface. A PDFium binding is
not the same as a small self-contained engine. Treat it as a later measured
option, not a first dependency.

### OCR

Tesseract and Leptonica are common open-source choices, but OCR language data,
native binaries, and model distribution must be reviewed separately. Make OCR
an optional provider and keep ordinary editing independent of it.

## Recommended architecture

The safest practical design is a layered editor:

1. **Document model:** page tree, metadata, permissions, object references,
   source hashes, and a non-destructive operation history.
2. **Renderer adapter:** PDF.js in a web UI or a measured native renderer.
3. **Structure adapter:** page reorder/rotate/crop/merge/split, metadata,
   attachments, forms, and save validation.
4. **Overlay layer:** annotations, text boxes, shapes, stamps, highlights,
   signatures-as-visual-appearance, and redaction preview.
5. **Commit/export layer:** generates a new PDF, preserves the source until
   atomic publication succeeds, and records what was flattened or changed.
6. **Optional providers:** OCR, QPDF operations, digital signatures, and AI.

Do not begin with arbitrary in-place text editing. PDF text is positioned drawing
content, not a normal document flow. A safe first editor can add and edit
overlay text boxes, while preserving original content. Later, implement
limited text replacement only when font metrics, encoding, clipping, and line
geometry are proven by fixtures.

## Feature staging

### Stage 1: reliable viewer and page workspace

- open/save-as without modifying the source;
- page thumbnails, zoom, fit modes, search, text selection, print preview;
- page reorder, rotate, delete, insert blank page, split, merge;
- metadata read/edit and undoable operation history;
- dark, blue, and high-contrast themes with readable controls.

### Stage 2: non-destructive authoring

- text boxes and free text overlays;
- highlight, underline, strikeout, shapes, arrows, freehand ink, stamps;
- images and signature appearances as overlays;
- forms: fill existing AcroForms before building new field authoring;
- flatten overlays only through an explicit export command.

### Stage 3: security-sensitive features

- redaction that removes underlying text/images from the output object graph,
  followed by search/stream/visual verification;
- encryption/password handling through a reviewed engine;
- digital signatures using a documented CMS/PKCS#7/X.509 design;
- signature invalidation warnings after any post-sign edit;
- PDF/A validation as a report, not a marketing checkbox.

### Stage 4: optional providers

- OCR with separately licensed language data;
- compression profiles and image downsampling;
- Bates numbering;
- local AI summary/extraction behind an explicit local endpoint and privacy
  setting.

## Interface direction

Use a familiar but independently designed desktop layout:

- top command/ribbon area for Open, Save As, Undo, page operations, annotate,
  forms, redact, sign, export, and search;
- left thumbnail/navigation pane;
- center page canvas with continuous or single-page modes;
- right properties/tool pane;
- bottom page/zoom/status bar;
- keyboard shortcuts, command search, and accessible focus order.

Do not imitate PDF Gear or Nitro Pro logos, icons, exact measurements, wording,
or distinctive visual assets. Borrow common information architecture only.

## Portability decision

There is no single small executable that gives identical native UI and full PDF
coverage on every OS. Two viable tracks are:

### Track A: fastest cross-platform prototype

Rust core plus Tauri/PDF.js frontend. This gives a productive UI and broad
platform reach, but depends on the OS WebView and adds JavaScript assets. It is
portable only when the target system has a compatible WebView/runtime.

### Track B: strict native packaging

Rust or C++ native UI plus a measured renderer/backend. This can produce a
smaller, more self-contained Windows ZIP, but cross-platform UI work and PDF
rendering integration take longer. MuPDF licensing and PDFium size must be
resolved before choosing either.

Recommended project sequence: prototype the document model and page operations
with a small permissive component set, then measure a PDF.js/Tauri prototype
against a native-renderer prototype before committing to a framework.

## Legal and provenance rules

This is not legal advice. For every dependency, font, icon, sample PDF, model,
or code fragment, record exact source, version/commit, hash, license,
copyright/NOTICE requirements, static/dynamic linking implications, and
redistribution terms. Do not copy PDF Gear, Nitro Pro, VueScan, PDF.js viewer
assets, or other project source without a compatible license review. Use public
behavior and documented standards, then write independent code. Keep AGPL,
GPL, and commercial-only components out of the default product unless the
distribution strategy expressly supports them.

## Official references checked

- PDF.js: https://github.com/mozilla/pdf.js
- PDF.js license: https://github.com/mozilla/pdf.js/blob/master/LICENSE
- PDF.js architecture: https://mozilla.github.io/pdf.js/getting_started/
- pdf-lib: https://github.com/Hopding/pdf-lib
- QPDF: https://qpdf.sourceforge.io/
- QPDF license: https://qpdf.readthedocs.io/en/stable/license.html
- MuPDF licensing: https://mupdf.com/releases
- Existing project prompt: `prompt.txt`
