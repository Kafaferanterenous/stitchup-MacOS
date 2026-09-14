# Stitchup — Stage 0: Engine Interfaces

Date: 2026-09-10 (Australia/Adelaide)
Status: design-only. Interface contracts written independently; no engine code
has been downloaded, installed, or executed.

## 1. Purpose and rules

The UI and model never import a specific PDF engine. All engine access is
through narrow interfaces below. A conforming adapter is swappable without UI
changes, which is what lets us later compare PDF.js vs native renderers.

Rules:

- Signatures below are language-neutral (pseudocode). The actual binding is
  chosen during the portability experiment and recorded in the ledger.
- Every method is synchronous *or* speaks async in a single documented
  convention per adapter; long operations must not block the UI thread.
- Adapters return model types from the document model doc, never raw engine
  types — except where an opaque handle is explicitly noted `opaque`.
- Errors are raised as structured `EngineError { code, message, detail }`,
  never bare strings. Codes: `open_failed`, `encrypted`, `password_required`,
  `malformed`, `unsupported`, `permission_denied`, `io_error`, `cancelled`,
  `atomic_publish_failed`, `validation_failed`.
- No adapter may execute JavaScript embedded in a PDF, resolve external
  references, or write to disk except through the commit interface.

## 2. OpenDocumentAdapter

Used by Stage 1 file dialog and startup.

```
OpenDocumentAdapter
  open(source: SourceRef) -> OpenResult
  // OpenResult = { doc: Document, warnings: Warning[] }
  //   or EngineError(open_failed | encrypted | password_required | malformed)

  openWithPassword(source, password) -> OpenResult
  // passwords are transient; never stored on the model

  verifyIntegrity(doc) -> IntegrityReport
  // { sha256: Hex, size, pages, warnings: Warning[] }
```

Conformance requirements:

- The caller supplies `SourceRef { path, bytes? }`. The adapter MUST compute the
  SHA-256 of the original bytes and set `Document.sourceHash`.
- The adapter MUST report (not hide) warnings for encrypted docs, incremental
  updates, XFA, embedded JS, and odd page trees.
- On `password_required`, the adapter must NOT have written or altered anything.

## 3. RendererAdapter

Handles page drawing, thumbnails, text layer for selection/search, and pixel
metrics needed by the overlay layer.

```
RendererAdapter
  bind(document) -> RenderSession
  renderPage(session, pageNumber, scale, mode: page|thumb) -> RenderedPage
  // RenderedPage = { image: RenderBuffer, sizePx, sizePt, rotation }
  // cancelable + progress callback where supported

  getTextRuns(session, pageNumber) -> TextRun[]
  // TextRun = { rect, string, unicode, fontTag?, charRects[] }
  // used for selection + search highlight; NOT used for in-document edits

  search(session, query, opts) -> SearchResult[]
  // SearchResult = { pageNumber, rect, matchText }

  getFontMetrics?  // deferred: only needed for Stage 2 limited text ops
```

Conformance requirements:

- Rendering to the screen and to thumbnails uses the same code path.
- Rotation/zoom are pass-through params; the adapter handles page rotation at
  draw time so model coordinates stay stable.
- Rendered images must carry the pixel size they were asked for (DPI-backed);
  the UI computes CSS size, never the adapter.
- Search must match over `getTextRuns`, not over OCR, in Stage 1.

## 4. PageTreeAdapter (structure operations)

All page-level mutations route through this adapter. Nothing else reorders,
deletes, inserts, rotates, splits, or merges.

```
PageTreeAdapter
  reorder(doc, newOrder: PageNumber[]) -> Operation
  rotate(doc, pageNumbers, degrees: 0|90|180|270, direction: CW|CCW) -> Operation
  delete(doc, pageNumbers) -> Operation
  insertBlank(doc, afterPage, sizePt) -> Operation
  split(doc, pageNumbers) -> SplitResult   // one file per extracted page set (via export)
  merge(doc, otherDocs: Document[]) -> Operation
  extract(doc, pageNumbers) -> outSource   // model-only until commit
  getPage(doc, pageNumber) -> Page

  apply(doc, op: Operation) -> void      // applies + validates invariants
  revert(doc, op: Operation) -> void     // undo support
```

Conformance requirements:

- Every mutation returns an `Operation` that `revert` can reverse exactly.
- Every `apply` ends by re-validating Document invariants (>=1 page, ordering,
  hash unchanged).
- `split`/`merge`/`extract` manipulate the model; nothing touches disk until
  `CommitAdapter.export`.

## 5. MetadataAdapter

```
MetadataAdapter
  get(doc) -> Metadata
  set(doc, updates: MetadataPatch) -> Operation
  clearCustom(doc, keys: string[]) -> Operation
```

Conformance: `set` only changes the model; export rewrites the Info dictionary.

## 6. CommitAdapter (save-as / export)

The only class allowed to touch the filesystem.

```
CommitAdapter
  export(doc, target: TargetRef, opts: ExportOptions) -> CommitResult
  // TargetRef = { path, overwrite: bool }
  // ExportOptions = { includeOverlays: bool, flattenOverlays: bool,
  //                   includeForms: bool, linearize: bool }
  // CommitResult = { newSha256, sizeBytes, wroteTemporaries, publishedPath }

  validate(doc, target) -> ValidationReport
  // before writing: model invariants + target path safety
  exportAccessCheck(clean: doc, currentPermissions) -> PermissionReport
```

Conformance requirements (source preservation + atomicity):

1. Never write to `Document.source` path. `export` requires an explicit target.
2. Write to a temporary file in the same directory as the target (same volume,
   so rename is atomic), then `fsync`, then atomically rename over the target.
3. If the target equals the source path is NOT possible — the UI must already
   have blocked that; the adapter double-checks and refuses with
   `permission_denied`.
4. On any failure the temporary file is deleted and the target left untouched
   (`atomic_publish_failed` otherwise; cleanup MUST still run).
5. `newSha256` is computed over the published bytes.
6. Validation runs BEFORE the temp write. A failed validation writes nothing.
7. Overlays are included only when `includeOverlays` is set; `flattenOverlays`
   is Stage 2 and must be explicit in the UI (never the default).

## 7. ProviderRegistry (optional Stage 4 shape)

Kept as a forward interface so Stage 1 doesn't hard-code OCR/compression/AI:

```
ProviderRegistry
  register(kind: provider_kind, provider: Provider) -> void
  has(kind) -> bool
  get(kind) -> Provider | null
```

Stage 1 ships with the registry present but empty, and the UI disables any
feature whose provider is absent.

## 8. UI contract with adapters

- The UI renders from `RendererAdapter` output and edits exclusively through
  `PageTreeAdapter`/`MetadataAdapter`/overlay ops.
- The UI never calls `CommitAdapter.export` without first showing a
  confirmation that names the exact target path.
- Keyboard shortcuts call the same command layer as the buttons: a
  `CommandBus { execute(cmd), undo(), redo() }` that wraps adapter calls in
  Operations.

## 9. Language binding decision (record here when chosen)

(to be filled after the portability experiment; see ledger for status)
- Container process: Rust | JS | other
- Renderer binding: PDF.js in-WebView | native
- Structural binding: pdf-lib | QPDF | native
- Justification + measurements: TBD