# Stitchup — Stage 0: Dependency / Provenance Ledger + Data-Safety Rules

Date: 2026-09-10 (Australia/Adelaide)
Status: ledger v1. Approved-for-use entries are the Stage 1 baseline. Nothing
here has been downloaded, installed, built, or executed yet. SHA-256 values are
**to be recorded at download time** and are intentionally blank until then.

## 1. Rules (applies to every component, font, icon, fixture, model, codec)

1. No dependency enters the build without an approved line in this ledger.
2. `Approval` = "open" (candidate) or "approved" (someone recorded version +
   license + provenance and chose to adopt). This document v1 approves the
   Stage 1 baseline in §3.
3. At the moment something is downloaded/pinned, fill in: exact version/commit,
   SHA-256 of the artifact, expected size, download source URL, verification
   method, and licenses/NOTICE/linkage requirements. Record every download in
   `dev_notes.txt` too.
4. Copyleft (GPL/AGPL/LGPL) components are excluded from the default build
   unless this ledger entry is explicitly promoted by a project owner.
5. AGPL/commercial-only components (MuPDF, Ghostscript), open-core code
   (Stirling-PDF `proprietary/`), and Fair-Core code (EmbedPDF `cloudpdf/server`)
   are never bundled; they may only be studied as behavior references.
6. Keep the gate list in `spec/pdf_editor_ai_handoff_prompt.txt` (redaction,
   signatures, encryption, OCR, PDF/A) — a gate does not open by adding a line
   here; it opens only after a dedicated review in the relevant Stage doc.
7. Third-party notices: each approved independent package that requires an
   acknowledgement ships its LICENSE/NOTICE text into a `NOTICES/` tree that the
   About → Licenses modal displays. Exact linking obligation (static/dynamic/
   JS bundle) is recorded per entry.
8. This is engineering governance, not legal advice. Stop and report uncertainty
   instead of guessing.

## 2. Data-safety rules (source preservation + atomic save-as)

From `docs/STAGE0_ENGINE_INTERFACES.md` §6, restated here so the ledger is
self-contained:

1. The app never writes to the path it opened. "Save" always means "export to an
   explicitly chosen target path."
2. Export writes a temporary file in the target directory (same volume), fsyncs
   it, validates byte size, then atomically renames over the target.
3. On any failure the temp file is deleted; the target and the source are
   untouched.
4. A successful save records the new SHA-256 in the commit result and in the
   operation history; it never replaces `Document.sourceHash`.
5. The UI must warn before any export that will overwrite an existing file, and
   must refuse to save directly onto the opened source path.
6. Untrusted content rules from the model doc: no JavaScript execution, no
   external network resolution, no auto-open of embedded files.

## 3. Stage 1 baseline — ledger v1 (approved)

All sizes/versions that were web-verified on 2026-09-10 are recorded; where a
crate/package is "pick latest stable", the exact version + SHA-256 + size is
filled in the moment it is downloaded. License identities were taken from the
projects' own repos/LICENSE pages on 2026-09-10.

| # | Component | Purpose | License | Source | Verified (2026-09-10) | Approve |
|---|---|---|---|---|---|---|
| D-01 | Tauri 2 (Rust shell) | Desktop shell, IPC, native dialogs, atomic save via Rust command | MIT / Apache-2.0 (dual, project) | github.com/tauri-apps/tauri; crates.io `tauri` | current stable 2.x | **approved** (Track A) |
| D-02 | React 19 + TypeScript + Vite | UI layer | MIT (React) / Apache-2.0 (Vite? TS) | npm | pick stable | **approved** |
| D-03 | pdfjs-dist (PDF.js) | renderer + text layer + search | Apache-2.0 (docs CC BY-SA 2.5; dist carries notices) | github.com/mozilla/pdf.js | pick stable | **approved** |
| D-04 | pdf-lib | page ops, forms, overlays compile, metadata, export | MIT (+ @pdf-lib/fontkit) | github.com/Hopding/pdf-lib | 1.4.0 | **approved** |
| D-05 | lopdf (Rust) | structural normalize, decrypt/encrypt, object streams | MIT (except bundled Montserrat font — do not use that font) | github.com/J-F-Liu/lopdf | 0.44.0; **pin >= 0.42.0** (RUSTSEC-2026-0187 fixed in 0.42.0) | **approved** |
| D-06 | Konva + react-konva | overlay/annotation canvas | MIT | github.com/konvajs/konva | pick stable | **approved** |
| D-07 | perfect-freehand | smooth freehand ink | MIT | github.com/steveruizok/perfect-freehand | pick stable | **approved** |
| D-08 | dnd-kit | thumbnail drag/reorder | MIT | github.com/dnd-kit/... | pick stable | **approved** |
| D-09 | @tauri-apps/cli, plugins (dialog, fs, path) | build + native features | MIT/Apache-2.0 | tauri.app plugins registry | stable | **approved** |

### Held in pending (candidate, NOT yet approved for use)

| # | Component | Purpose | License | Why held |
|---|---|---|---|---|
| D-10 | @embedpdf/pdfium | WASM PDFium renderer | MIT + PDFium Apache-2.0 | Renderer decision deferred to Track A vs B experiment; exclude `cloudpdf/server`. |
| D-11 | pdfium-render + PDFium binary | native renderer (Track B) | MIT OR Apache-2.0 + BSD-3/Apache (PDFium) | Needs the dedicated PDFium notice review; bblanchon binaries MIT scripts; WASM non-growable-heap caveat. |
| D-12 | QPDF / qpdf-wasm | repair, linearize, structural transforms | Apache-2.0 / ISC | Only if Stage 1 contract unmet by JS+Rust path. Linkage (native sidecar) must be assessed. |
| D-13 | pikepdf | redaction, sanitizer, linearize | MPL-2.0 (file-level copyleft) | Stage 3 redaction gate; pulls Python + QPDF; decide Rust-lopdf path vs Python sidecar first. |
| D-14 | pdfminer.six | text runs + font metrics/encodings | MIT | Stage 2 limited text-edit; requires Python sidecar. |
| D-15 | @signpdf/signpdf + @signpdf/placeholder-pdf-lib + @signpdf/signer-p12 + node-forge | PAdES/PKCS#7 signing | MIT / BSD-3 (node-forge) | Stage 3 signature gate; verify PKCS#12/X.509 assumptions with fixtures first. |
| D-16 | pyHanko | signing/validation (Python) | MIT | Alternate to D-15; only if Python sidecar is adopted. |
| D-17 | tesseract.js + tessdata | OCR | Apache-2.0 (+ per-language data) | Stage 4 OCR gate; record language data files, version, hash separately. |
| D-18 | fabric.js | alternate overlay canvas | MIT | Only if Konva fails overlay acceptance; do not take both in default build. |

### Known gaps (no permissive off-the-shelf answer yet — design, don't assume)

- True in-place editing of *existing* text runs (partial). Postpone to Stage 2
  "limited text replacement" after font-metric fixtures pass.
- Certified/`fillin` signature fields + PAdES LTA timestamping profiles.
- PDF/A validation other than a structural report.
- Compression / image downsampling profiles (Stage 4; Ghostscript excluded —
  prefer qpdf-wasm or custom stream filters).
- Color profile / spot color handling.

## 4. Commandments for anyone adding a dependency

Copy this block into the `dev_notes.txt` entry:

```
DEPENDENCY_ADDED <name> <version>
source=<url>
sha256=<to be filled at download>
size=<to be filled>
license=<spdx>
notices=<required? where shipped>
linkage=<static|dynamic|bundled-js|sidecar>
modification_scope=<none|patched? describe>
decision=<approved|candidate+why>
created=<date>
```

## 4b. Actually downloaded for the Rust core (2026-09-10, resolved by Cargo)

Registries returned these exact versions; `sha256` = checksum recorded in
`source/core/Cargo.lock` (single published checkout, canonical cargo checksum).
The crate did not vendor any NOTICE-requiring assets (lopdf bundles a Montserrat
font: **do not use that font**; skip its font file when shipping). No source files
were modified.

```
DEPENDENCY_ADDED lopdf 0.44.0
source=crates.io (github.com/J-F-Liu/lopdf tag 0.44.0)
sha256=5e2ec995d822e05cabc3f06d196ee43650af3fe4fe38012cacb35e0c3d113b68
license=MIT (src/encodings/font - bundled Montserrat; omit that font)
linkage=static (Rust lib)
modification_scope=none
decision=approved (D-05; >=0.42.0 pin honored)
created=2026-09-10
```

```
DEPENDENCY_ADDED sha2 0.10.9 (direct) / 0.11.0 (transitive)
source=crates.io (RustCrypto/hashes)
sha256=a7507d819769d01a365ab707794a4084392c824f54a7a6a7862f8c3d0892b283 (0.10.9)
license=Apache-2.0 OR MIT
linkage=static
modification_scope=none
decision=approved (hash integrity)
created=2026-09-10
```

```
DEPENDENCY_ADDED flate2 1.1.10
source=crates.io (rust-lang/flate2-rs)
sha256=6e634e2e0ebac1ee034020da1ca582e17ffe4e0f5e985823721e168928136dcb
license=Apache-2.0 OR MIT
linkage=static
modification_scope=none
decision=approved (stream/stats fixtures)
created=2026-09-10
```

Transitive OS-ish crates (crc32fast 1.5.1, miniz_oxide 0.9.1, adler2 2.0.1,
plus log/iterators under lopdf) all license Apache-2.0/MIT (crates.io).
Full tree is in `Cargo.lock`.

## 4c. npm frontend deps (2026-09-14, resolved from registry on this macOS)

`sha256` = SHA-256 of each published tarball downloaded fresh from the npm
registry at the resolved version (computed with `shasum -a 256`). The registry
`integrity` field (sha512-base64) for every package is committed in
`source/app/package-lock.json` (97 packages total; `npm ci` reproduces the
exact tree). No sources were modified; esbuild's postinstall native-binary
build was explicitly approved. `@tauri-apps/plugin-path` does not exist on npm
(path permissions are `core:path:default` in Tauri 2) — removed from
package.json before install. pdf-lib resolved to 1.17.1 (ledger §3 said 1.4.0;
1.17.1 is the current 1.x line, MIT, same approved component).

```
DEPENDENCY_ADDED pdfjs-dist 4.10.38       (D-03)
source=registry.npmjs.org (github.com/mozilla/pdf.js)
sha256=1011b38553532d7078c59f26b15a471f8dae00f101b60e2add9b8511737a1ce0
license=Apache-2.0
linkage=bundled-js
modification_scope=none
decision=approved (D-03)
created=2026-09-14
```

```
DEPENDENCY_ADDED pdf-lib 1.17.1           (D-04)
source=registry.npmjs.org (github.com/Hopding/pdf-lib)
sha256=a7cc1eaf12e41e612a7be581162a63b18118aefc01e90f6a1f35347b1f324a1c
license=MIT (+ @pdf-lib/fontkit, bundled)
linkage=bundled-js
modification_scope=none
decision=approved (D-04; ledger version refreshed 1.4.0 -> 1.17.1)
created=2026-09-14
```

```
DEPENDENCY_ADDED react 19.3.0             (D-02)
source=registry.npmjs.org (github.com/facebook/react)
sha256=5a4024110c49476e1538a70fd65b9e59d038fcc280b78eb933c24e10453afbf9
license=MIT
linkage=bundled-js
modification_scope=none
decision=approved (D-02)
created=2026-09-14
```

```
DEPENDENCY_ADDED react-dom 19.3.0         (D-02)
source=registry.npmjs.org (github.com/facebook/react)
sha256=dbeffb1e6cdbbbba07f799da4635333d0e20c210a32278dded85441ec4cb6a17
license=MIT
linkage=bundled-js
modification_scope=none
decision=approved (D-02)
created=2026-09-14
```

```
DEPENDENCY_ADDED konva 9.3.22             (D-06)
source=registry.npmjs.org (github.com/konvajs/konva)
sha256=85931a2baf6afd7b1e3b4fe2635e1ad2265064fa568de5d739faca68828ad9e9
license=MIT
linkage=bundled-js
modification_scope=none
decision=approved (D-06)
created=2026-09-14
```

```
DEPENDENCY_ADDED react-konva 18.2.16      (D-06)
source=registry.npmjs.org (github.com/konvajs/react-konva)
sha256=8919709212aa4067ff9e7542fcc39565f4f4594430bc512ce28ddebc8744c0c7
license=MIT
linkage=bundled-js
modification_scope=none
decision=approved (D-06)
created=2026-09-14
```

```
DEPENDENCY_ADDED perfect-freehand 1.2.3   (D-07)
source=registry.npmjs.org (github.com/steveruizok/perfect-freehand)
sha256=060048047071b6ffb2170c8b867ba995f365ec811dc32435574d3d5f7625981f
license=MIT
linkage=bundled-js
modification_scope=none
decision=approved (D-07)
created=2026-09-14
```

```
DEPENDENCY_ADDED @dnd-kit/core 6.3.1      (D-08)
source=registry.npmjs.org (github.com/dnd-kit/core)
sha256=f68d69af50234872c2e3345bc8bf98d8bcb0a9481eab6d370605c99e76a98c33
license=MIT
linkage=bundled-js
modification_scope=none
decision=approved (D-08)
created=2026-09-14
```

```
DEPENDENCY_ADDED @dnd-kit/sortable 8.0.0  (D-08)
source=registry.npmjs.org (github.com/dnd-kit/sortable)
sha256=a6b456b38c0acbbdced158f8690ff1ca4df7e703ea469b897a73a17c1ba47850
license=MIT
linkage=bundled-js
modification_scope=none
decision=approved (D-08)
created=2026-09-14
```

```
DEPENDENCY_ADDED @dnd-kit/utilities 3.2.2 (D-08)
source=registry.npmjs.org (github.com/dnd-kit/utilities)
sha256=048c7f56d17ceae19a29979700582566c0036600ee4d2c38ba13e9e8f906a18e
license=MIT
linkage=bundled-js
modification_scope=none
decision=approved (D-08)
created=2026-09-14
```

```
DEPENDENCY_ADDED @tauri-apps/api 2.11.1   (D-01)
source=registry.npmjs.org (github.com/tauri-apps/tauri)
sha256=70a6defdbffbe388b3c2715a717bbacbd885c6ceba8b4ac69051a9794a939e0a
license=Apache-2.0 OR MIT
linkage=bundled-js (IPC to the Rust host)
modification_scope=none
decision=approved (D-01)
created=2026-09-14
```

```
DEPENDENCY_ADDED @tauri-apps/plugin-dialog 2.7.3   (D-09)
source=registry.npmjs.org (github.com/tauri-apps/plugins-workspace)
sha256=e573c7dd2f68399349ae1e573546dda6585985f6f856bd085330dea30382b9ea
license=MIT OR Apache-2.0
linkage=bundled-js (+ Rust side, tauri-plugin-dialog 2.7.3)
modification_scope=none
decision=approved (D-09)
created=2026-09-14
```

```
DEPENDENCY_ADDED @tauri-apps/plugin-fs 2.5.2       (D-09)
source=registry.npmjs.org (github.com/tauri-apps/plugins-workspace)
sha256=8fdf77b07f0381c463c2f3bc4ce3a02a93bd6c42bed0a40a8316e08e9416d993
license=MIT OR Apache-2.0
linkage=bundled-js (+ Rust side, tauri-plugin-fs 2)
modification_scope=none
decision=approved (D-09)
created=2026-09-14
```

Build toolchain (devDependencies, same provenance):

```
DEPENDENCY_ADDED @tauri-apps/cli 2.11.4            (D-09)
source=registry.npmjs.org (github.com/tauri-apps/tauri)
sha256=16710beec04ee6be052d40276b7b2b7f4de0d8bee2138df94dbf6d6d8014028f
license=Apache-2.0 OR MIT
linkage=sidecar binary (per-platform @tauri-apps/cli-*)
modification_scope=none
decision=approved (build tool)
created=2026-09-14
```

```
DEPENDENCY_ADDED vite 6.4.3 / typescript 5.6.3 / @vitejs/plugin-react 4.7.0   (D-02 toolchain)
source=registry.npmjs.org (vitejs/vite, microsoft/TypeScript, vitejs/vite-plugin-react)
sha256=vite  e498f166131407cd8ab819dfbc656c62d514b4e5e62f6f639efcef1614beb809
license=vite MIT; typescript Apache-2.0; plugin-react MIT
linkage=bundled-js / build
modification_scope=none
decision=approved (D-02)
created=2026-09-14
```

```
DEPENDENCY_ADDED @types/react 19.3.0 / @types/react-dom 19.3.0 / @types/pdf-parse 1.1.5
source=registry.npmjs.org (DefinitelyTyped)
sha256=@types/react ffa77ffdbdc82e5f67a0fd74d750990375a7dfd8685ec7f10a27c0db128f7766
license=MIT
linkage=dev-only type defs
modification_scope=none
decision=approved (typechecking)
created=2026-09-14
```

```
DEPENDENCY_REMOVED @tauri-apps/plugin-fs 2.5.2 + tauri-plugin-fs (direct)   (D-09)
reason=unused. Neither the frontend nor the Rust command layer touches files
       through the fs plugin: all file I/O goes through the IPC command layer
       (std::fs in commands.rs), and the webview only PICKS paths via the
       dialog plugin. Keeping `fs:default` granted access to a plugin whose
       commands were never registered — dead surface. Least-privilege removal.
scope=source/app/package.json                 (-@tauri-apps/plugin-fs)
      source/app/src-tauri/Cargo.toml          (-tauri-plugin-fs direct dep)
      source/app/src-tauri/src/lib.rs          (-.plugin(tauri_plugin_fs::init()))
      source/app/src-tauri/capabilities/default.json
                                               (-"fs:default", -"core:path:default")
note=tauri-plugin-fs stays in Cargo.lock as a TRANSITIVE crate of
      tauri-plugin-dialog, but is no longer registered, so no fs commands are
      exposed to the webview.
license=MIT OR Apache-2.0 (unchanged)
decision=approved (least privilege)
created=2026-10-13
```

Notes:
- Dev-only rendering/self-test inputs that are NOT shipped: the bundled
  pdfjs worker asset is already emitted by vite (dist/assets/pdf.worker-*.mjs).
- Re-verify each license/NOTICE at the pinned version before release (rule §0).

## 5. Status stamp

- Ledger v1 written on 2026-09-10. Approvals in §3 are the Stage 1 baseline.
- **Update 2026-09-10 (build session):** Rust core compiled and ran against
  real crates. §4b now records the actual resolved versions + Cargo.lock
  checksums for lopdf/sha2/flate2. The frontend deps (D-01..D-03,D-06..D-09)
  remain un-pinned: they are resolved at first `npm/pnpm` install by the
  platform build AIs (see AI_HANDOFF_WINDOWS.txt / AI_HANDOFF_LINUX.txt), who
  must copy the same DEPENDENCY_ADDED block shape into `dev_notes.txt`.
- **Update 2026-09-14:** frontend now installed and resolved on this macOS
  (Node 26.8.2 / npm 11.19.1). §4c lists direct npm deps + their shas; full
  resolvable tree committed in package-lock.json. `npm run build` (tsc + vite)
  passes; `npm run tauri dev` builds the Rust host and launches the app.
- **Update 2026-10-13:** removed the unused fs plugin (direct dep, host
  registration, and the `fs:default` capability) — no fs commands are registered,
  so the grant was dead. The dialog plugin is kept (open/save path pickers).
- Re-verify each license/NOTICE at the actual pinned version before release.