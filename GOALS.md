# Stitchup (macOS) — goals

Target: the best all-round PDF editor, at feature and look parity with the
Windows build (github.com/Kafaferanterenous/stitchup-Win v0.12.28), and ahead of it where macOS allows.
Updated 2026-11-14 (v0.6.0). Status key: ✅ done · 🔨 in progress · ⬜ to do. Every ✅ item is covered by an automated test.

## A. Look & navigation (same as Windows)
| # | Goal | Status |
|---|------|--------|
| A1 | Ribbon tabs Document · Pages · Annotate · Content · Zoom · Navigate · Security, Windows button set/labels/tooltips | ✅ |
| A2 | Menu bar File · Export · Edit · Pages · Annotate · View · Tabs · Help (native macOS menu, same items/shortcuts) | ✅ |
| A3 | The 9 Windows colour schemes (Light Grey default … Pastel 3), remembered | ✅ |
| A4 | Windows status bar (file name + `*` when unsaved, page x of y, selection count, zoom); red Save when dirty | ✅ |
| A5 | Drag & drop a PDF onto the window to open; open from Finder / command line — built; drag-drop needs a check on your Mac (not testable headless) | 🔨 |
| A6 | Multi-document tabs (⌘T, ⌘W, ⌃Tab / ⌃⇧Tab), each with its own page, zoom and undo; opening an open file switches to its tab; closing an unsaved tab asks | ✅ |

## B. Document
| # | Goal | Status |
|---|------|--------|
| B1 | New blank document | ✅ |
| B2 | Save in place (atomic) + Save As | ✅ |
| B3 | Import / Merge PDFs; Combine into New — Import, Merge PDFs, Combine into New | ✅ |
| B4 | Export Text / CSV / Images (PNG, JPG) / RTF / DOCX / ODT | ✅ |
| B5 | Print, Print Preview, Print to PDF — preview dialog + macOS print panel (Save as PDF there); not yet tried on a real printer | ✅ |
| B6 | Document Properties ✅, Attachments list + save | ✅ |
| B7 | Compress PDF — unused objects removed + streams/object streams compressed; image downsampling still to do | ✅ |
| B8 | Crash-recovery snapshots (Off / 30 s / 1 min / 5 min, off by default like Windows) offered back after a crash; Settings (⌘,); quitting or closing the window with unsaved changes asks | ✅ |
| B9 | Built-in User Guide (F1) — in-app guide (F1) rather than a help.pdf | ✅ |

## C. Pages
| # | Goal | Status |
|---|------|--------|
| C1 | Rotate / delete / add / reorder / multi-select | ✅ |
| C2 | Extract / Split | ✅ |
| C3 | Auto-Crop | ✅ |
| C4 | Move pages between open tabs (drag a thumbnail onto a tab, or right-click ▸ Move Page to …) | ✅ |

## D. Annotate
| # | Goal | Status |
|---|------|--------|
| D1 | Highlight, Underline, Strike-out (drag to draw) | ✅ |
| D2 | Sticky Note, Text Box | ✅ |
| D3 | Shapes (rectangle, oval, line, arrow), Pen (freehand ink) | ✅ |
| D4 | Stamps (Draft, Approved, Paid…), Links (web + page) and link activation — stamps (12 + custom), web links and page links; clicking a link follows it | ✅ |
| D5 | Select / move / delete annotations | ✅ |

## E. Content editing
| # | Goal | Status |
|---|------|--------|
| E1 | In-place text edit in the original font | ✅ |
| E2 | Unicode fallback font (Polish ą ł etc. when the original font lacks them) — Central European Latin (Polish ą ł ż…), Baltic, Turkish, Romanian; Cyrillic/Greek/CJK still to do | ✅ |
| E3 | Select / move / delete content objects (text, images, paths), marquee select — text runs and images (vector paths not yet selectable); move, delete, marquee, ⌘-click | ✅ |
| E4 | Recolor, Font & size, Fore/Back colours — recolour text, font family/bold/italic/size, Fore/Back colours, vector shapes into the page | ✅ |
| E5 | Insert image — PNG (with transparency), JPEG (embedded as-is), GIF/WebP/HEIC/TIFF | ✅ |

## F. Security & forms
| # | Goal | Status |
|---|------|--------|
| F1 | Open password-protected PDFs | ✅ |
| F2 | Encrypt (AES-256 — stronger than the Windows RC4) | ✅ |
| F3 | Watermark | ✅ |
| F4 | True redaction (content removed, not covered) — text glyphs and whole images under the box removed; partly-covered images are blacked out | ✅ |
| F5 | Fill AcroForms — text, multi-line, checkbox, radio, drop-down; appearances regenerated (shows in Preview/Acrobat); undoable | ✅ |
| F6 | Digital signature — self-signed RSA-2048 / SHA-256 PKCS#7, visible box; verified with OpenSSL. Signing an already-signed, unchanged PDF appends an incremental update, so every earlier signature stays valid (two signatures verified with OpenSSL) | ✅ |

## G. Beyond Windows (macOS)
| # | Goal | Status |
|---|------|--------|
| G1 | OCR with Apple Vision, on-device (Content ▸ OCR: pages without text / this page / all) → invisible text layer, so scans become searchable and copyable; one undo step | ✅ |
| G2 | Find with F3 / ⇧F3, highlight all ✅ | ✅ |
