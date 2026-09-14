# Stitchup — Stage 0: Clean-Room Document Model

Date: 2026-09-10 (Australia/Adelaide)
Status: design-only. Nothing here has been implemented, downloaded, or run.

This document is the single source of truth for Stitchup's in-memory document
model. It is written independently from the behavior of the public PDF
specification (ISO 32000-1/-2) and from the general information architecture of
common PDF applications. It does not copy any project source.

## 1. Design principles

1. **The source PDF is sacred.** Stitchup opens a PDF and never mutates the file
   it read from. All work happens in memory against a model, and the only write
   happens through the commit/export layer into a new file (or a new path).
2. **Non-destructive by default.** Every supported operation appends to an
   operation history instead of rewriting the model in place. An operation can
   always be undone by replaying history minus the last entry.
3. **No layering of vendor concepts.** The model represents *what a document
   is* (pages, objects, metadata, permissions), not *how a specific engine
   stores it*. Engine-specific details live behind adapters.
4. **Untrusted input stays untrusted.** Parsed pages may carry scripts, links,
   embedded files, and forms. The model marks these as untrusted and the viewer
   never executes them.

## 2. Top-level model

```
Document
├── source            // SourceHandle: path, bytes? SHA-256, size, mtime, engine tag
├── catalog           // essential catalog fields the app cares about
├── pages             // ordered Page[] (sorted page tree leaves)
├── metadata          // Metadata (title, author, subject, keywords, creator, producer, dates)
├── permissions       // Permissions (user/owner password state, print/copy/edit flags)
├── form              // AcroForm (fields, sig flags) or null
├── attachments       // EmbeddedFile[]
├── structure         // optional outline/bookmark tree, tagged structure info
├── sourceHash        // SHA-256 of original bytes at open time
└── history           // OperationHistory (undo stack)
```

### 2.1 Document invariants

- A `Document` always has at least one `Page` after open or construction.
- `pages` order is the rendered/saved order. Reordering only ever reorders this
  array; the underlying source is untouched until commit.
- `sourceHash` never changes for the lifetime of the model. Any save-as that
  succeeds produces a *new* hash on disk, never mutates this field.
- The model is valid (all invariants hold) immediately after open and after
  every committed operation.

## 3. Page model

```
Page
├── pageNumber        // 1-based, derived from position in Document.pages
├── size              // Size { widthPt, heightPt } in PDF points (1/72 in)
├── rotation          // int, one of 0, 90, 180, 270 (degrees clockwise)
├── labels            // optional PageLabel // e.g. roman, numbered "A-1"
├── content           // PageContent (the drawing stream reference + counters)
├── resources         // ResourceRef (font/color profile references; opaque to model)
├── annotations       // AnnotOverlay[]  (non-destructive additions made in Stitchup)
├── native           // NativeAnnot[]   (annotations that existed in the source)
├── untrusted         // untrusted-content flags (scripts, links, forms, media)
└── boxes             // MediaBox (required), CropBox, BleedBox, TrimBox, ArtBox (optional)
```

Notes:
- Coordinates are in PDF points in the page's default rotated space; the model
  stores them un-normalized and the renderer adapter handles rotation at draw
  time.
- `PageContent` is a reference into the source object graph, not copied text.
  The model does not attempt to interpret text strings at this layer; that is
  the renderer/export layer's job.

### 3.1 Overlay vs native annotations

- **Native annotations** exist in the source PDF. The model preserves them and
  reports them read-only (unless a later stage adds full annotation editing).
- **Overlay annotations** (`AnnotOverlay`) are a Stitchup concept: anything the
  user adds (text box, highlight, shape, ink, stamp, image, signature
  appearance). They are kept in normalized model form so they can be planted by
  the structural adapter into the exported PDF.

## 4. Operation history

```
OperationHistory
├── entries           // Operation[]  (oldest → newest)
└── cursor            // current position (for undo/redo)
```

```
Operation
├── id                // uuid
├── kind              // enum: reorder | rotate | delete | insert | extract |
│                     //       split | merge | metadata | overlay_* | save_as
├── label             // human-readable, localized later
├── before            // ModelState snapshot/ref needed to reverse
├── after             // ModelState snapshot/ref needed to re-apply
├── appliedAt         // ISO-8601 timestamp
└── undoInfo          // adapter-specific reverse data (opaque)
```

Rules:
- `undo()` pops to `cursor - 1` and calls the page-tree adapter to rebuild.
- `redo()` moves to `cursor + 1`.
- `undo`/`redo` never touch the file on disk.
- A `save_as` is recorded as a history entry but is not itself undoable in the
  document (it is a bookmark for the export log; undoing it only records that
  the last save target is no longer current).
- Closing without saving discards the model, not the source.

## 5. Metadata

```
Metadata
├── title, author, subject, keywords     // strings (optional)
├── creator, producer                     // strings (optional)
├── creationDate, modDate                 // ISO-8601 or null
├── custom                                // key→string map (optional)
└── raw                                        // preserved source dictionary (read-only)
```

Editing replaces display values in the model; the structural adapter rewrites
the document info dictionary **only** during export.

## 6. Permissions and security state

The model records what the source told us; it does not enforce anything by
itself. Enforcement happens at open/export time via the structural adapter.

```
Permissions
├── encrypted            // bool
├── encryptionMethod     // enum: none | rc4_40 | rc4_128 | aes_128 | aes_256 | unknown
├── ownerPasswordSet     // bool  (never store the password in the model)
├── userPasswordRequired // bool
├── allowPrint, allowCopy, allowEdit, allowEditAnnotations, allowFormFilling  // flags
└── wasOpenedWithRestrictions // bool (note shown to user, e.g. "print limited")
```

Security-sensitive operations (redaction, signing, encryption) are Stage 3+ and
never happen against the live source file.

## 7. Forms

```
AcroForm
├── fields            // FormField[]
├── needAppearances   // bool
├── calcOrder         // string[]
├── signatureFlags    // int (mask; read-only in Stage 1-2)
└── xfa               // present? treated as untrusted in Stage 1
```

```
FormField
├── name, altName    // strings
├── kind             // enum: text | checkbox | radio | dropdown | list | button | signature | unknown
├── rect             // Rect in page space
├── pageRef          // Page ref or null
├── value            // current value (string | bool | selection | null)
├── readOnly, required, multiline, password  // flags
├── options          // for dropdown/list
└── appearance       // existing appearance stream ref (preserve unless rebuilt)
```

Stage 1 only reports form presence and flags. Form filling is Stage 2.

## 8. Render/print state (not persisted)

The UI keeps presentation state outside the model: current zoom, fit mode,
selected page, search hits, scroll position, and theme. These are "view state"
and are never written into the exported PDF.

## 9. Engine tags

Every in-memory object created by an adapter carries an `engine` tag (e.g.
`pdf-js`, `pdf-lib`, `qpdf`, `stichup-native`) so:
- provenance is auditable in crash reports/handover notes, and
- the same model field can be produced by a different adapter in a later
  portability experiment without changing UI code.

The model layer defines the interfaces (next doc) and the adapters satisfy
them. The model never imports an engine type.

## 10. What this deliberately does NOT model (yet)

- Extracted text runs / font glyph geometry (Stage 1 brings a text layer for
  selection/copy through the renderer, not into the model).
- OCR results (Stage 4 provider).
- Bates numbering counters (Stage 4).
- PDF/A conformance state (Stage 3 report).
- Compression profiles (Stage 4).

Adding any of these happens by extending this document first, then the
interfaces, then the adapters — never the reverse.