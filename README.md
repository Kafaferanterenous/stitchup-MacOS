# Stitchup — Project 007

A fully featured, portable, offline-first, cross-platform PDF viewer/editor.
Independently designed — inspired by the general workspace of common PDF
applications (ribbon, thumbnail pane, canvas, properties pane, status bar), not
a copy of any commercial product.

Status: **Stage 0 (design contract) complete; Stage 1 core built and verified
on macOS.** The Rust structural core (`source/core`) compiles, its hash tests
pass, and all 13 fixtures are generated + verified (no Node/npm on this
machine, and by design the desktop frontend is built first on Windows/Linux —
see `AI_HANDOFF_WINDOWS.txt` / `AI_HANDOFF_LINUX.txt`). The Tauri 2 + React app
shell (`source/app`) is scaffolded end-to-end and compiles on the Rust side;
the Windows/Linux build AIs install frontend deps, build the installer, and
record npm deps in the ledger.

## Layout

```
007_2026_09_10_Stitchup/
├── spec/                      # original Project 021 handoff + research (immutable)
│   ├── prompt.txt
│   ├── pdf_editor_ai_handoff_prompt.txt
│   ├── RESEARCH_2026_08_20_PDF_EDITOR_ECOSYSTEM.md
│   └── dev_notes.txt
├── docs/
│   ├── STAGE0_DOCUMENT_MODEL.md
│   ├── STAGE0_ENGINE_INTERFACES.md
│   ├── STAGE0_DEPENDENCY_LEDGER.md
│   ├── STAGE0_FIXTURES_AND_ACCEPTANCE.md
│   └── RESEARCH_2026_09_10_GITHUB_ECOSYSTEM.md
├── AI_HANDOFF_WINDOWS.txt     # for the AI building the Windows packages
├── AI_HANDOFF_LINUX.txt       # for the AI building the Linux packages
├── fixtures/generated/        # sanitized fixture PDFs F-01..F-14 (F-09 deferred)
├── scripts/
│   ├── generate_fixtures.sh   # regenerate fixtures via the Rust core
│   └── verify_fixtures.sh     # run the acceptance-subset assertions
├── source/
│   ├── core/                  # stitchup-core Rust crate (lopdf) — BUILDABLE
│   └── app/                   # Tauri 2 + React/TS frontend + src-tauri host
├── dev_notes.txt              # live multi-AI journal (append-only)
└── NOTICES/                   # third-party license texts (created at build)
```

## Read first

1. `dev_notes.txt` (latest checkpoint)
2. `spec/pdf_editor_ai_handoff_prompt.txt` (project rules + stages + legal)
3. `docs/STAGE0_*.md` (model, interfaces, ledger, fixtures/acceptance)
4. `AI_HANDOFF_WINDOWS.txt` / `AI_HANDOFF_LINUX.txt` for those builds

## Ground rules (short version)

- **Legal:** no copying PDF Gear, Nitro/Jumbl, Adobe, PDF.js viewer assets, or
  any other project's code. Permissive deps only (MIT/Apache/MPL/BSD); AGPL,
  GPL, MuPDF, Ghostscript and open-core code stay out of the default build.
- **Data safety:** never overwrite the opened source; atomic temp+fsync+rename
  save-as only.
- **Stages:** 0 design (done) → 1 viewer/page-workspace → 2 overlays/forms →
  3 redaction/signing/encryption → 4 OCR/compression/AI (gated).
- **Journal:** every session appends to `dev_notes.txt`, never overwrites.

## Theming

Themes: XP-inspired (default), Dark, Blue — all with readable contrast for
buttons/menus (WCAG spot-checked). Independently styled, not pixel-matching any
commercial or retro-theme project's artwork.