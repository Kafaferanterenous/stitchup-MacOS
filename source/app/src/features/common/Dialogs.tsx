import { useEffect, useRef, useState, type ReactNode } from "react";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useAppState, store, getState, docName, isUntitled, rgb, annotColor } from "../../store";
import { extractTo, parsePages, splitInto } from "../../actions";
import { cmdDiscardRecovery, cmdListAttachments, cmdSaveAttachment, cmdSaveEncrypted, cmdSign, cmdWriteFile, type AttachmentInfo, type RecoveryEntry } from "../../ipc";
import { loadSettings, saveSettings, type Settings } from "../../settings";
import { FontPicker, fontName, useSystemFonts, type FontChoice } from "./FontPicker";
import { THEMES, applyTheme, currentTheme, type ThemeName } from "../../themes/theme";
import { pageImage, toBase64 } from "../../export";
import { PAPERS, fitOnSheet, printDocument } from "../../print";
import { renderPage } from "../../adapters/renderAdapter";

/** Shared modal shell: backdrop click and Escape close it. */
function Modal({ title, onClose, children, wide }: { title: string; onClose: () => void; children: ReactNode; wide?: boolean }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);
  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div className={`modal${wide ? " wide" : ""}`} role="dialog" aria-modal="true" aria-label={title} onMouseDown={(e) => e.stopPropagation()}>
        <h2>{title}</h2>
        {children}
      </div>
    </div>
  );
}

/**
 * Components shipped in the app (STAGE0_DEPENDENCY_LEDGER D-01..D-09, as
 * actually bundled). D-04/D-06/D-07 are approved for Stage 2 and are not yet
 * shipped code paths, so they are listed separately.
 */
const SHIPPED: { id: string; name: string; use: string; license: string }[] = [
  { id: "D-01", name: "Tauri 2", use: "desktop shell, native dialogs, IPC", license: "MIT / Apache-2.0" },
  { id: "D-02", name: "React 19, TypeScript, Vite", use: "user interface", license: "MIT / Apache-2.0" },
  { id: "D-03", name: "PDF.js (pdfjs-dist)", use: "page rendering, text layer, search", license: "Apache-2.0" },
  { id: "D-05", name: "lopdf", use: "PDF structure, text editing, save", license: "MIT" },
  { id: "D-08", name: "dnd-kit", use: "drag to reorder thumbnails", license: "MIT" },
  { id: "D-09", name: "Tauri dialog plugin", use: "open / save panels", license: "MIT / Apache-2.0" },
  { id: "—", name: "serde, base64, sha2, flate2 (Rust)", use: "data encoding, hashing, compression", license: "MIT / Apache-2.0" },
  { id: "—", name: "RustCrypto: rsa, x509-cert, cms, der, spki", use: "digital signatures (RSA-2048, X.509, PKCS#7)", license: "MIT / Apache-2.0" },
  { id: "—", name: "ttf-parser, subsetter", use: "installed fonts: read, subset and embed", license: "MIT / Apache-2.0" },
  { id: "—", name: "objc2 (app-kit, web-kit, foundation)", use: "native print settings", license: "MIT" },
  { id: "—", name: "Apple Vision (macOS)", use: "text recognition (OCR), on this Mac", license: "part of macOS" },
];
const NOT_YET: string[] = ["pdf-lib (D-04)", "Konva + react-konva (D-06)", "perfect-freehand (D-07)"];

export function AboutDialog() {
  return (
    <Modal title="About Stitchup" onClose={() => store.setDialog(null)} wide>
      <p>
        <b>Stitchup</b> — offline PDF viewer &amp; editor. Your files never leave this computer, and a file
        is only replaced when you save — written to a temporary file first, then swapped in, so it is never half-written.
      </p>
      <h3>Open-source components</h3>
      <table className="license-table">
        <thead>
          <tr>
            <th>Ledger</th>
            <th>Component</th>
            <th>Used for</th>
            <th>License</th>
          </tr>
        </thead>
        <tbody>
          {SHIPPED.map((c) => (
            <tr key={c.name}>
              <td>{c.id}</td>
              <td>{c.name}</td>
              <td>{c.use}</td>
              <td>{c.license}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="muted">Approved for later stages, not yet used: {NOT_YET.join(", ")}.</p>
      <div className="modal-actions">
        <button onClick={() => store.setDialog(null)} autoFocus>
          Close
        </button>
      </div>
    </Modal>
  );
}

export function ExtractDialog() {
  const app = useAppState();
  const max = app.doc?.pages.length ?? 0;
  const [range, setRange] = useState(`${app.currentPage}`);
  const pages = parsePages(range, max);
  async function go() {
    if (!pages) return;
    if (await extractTo(pages)) store.setDialog(null);
  }
  return (
    <Modal title="Extract pages" onClose={() => store.setDialog(null)}>
      <p className="muted">Save chosen pages as a new PDF. This document is not changed.</p>
      <label>
        Pages (e.g. 1-3, 5)
        <input value={range} onChange={(e) => setRange(e.target.value)} onKeyDown={(e) => e.key === "Enter" && void go()} autoFocus />
      </label>
      <p className={pages ? "muted" : "err-note"}>
        {pages ? `${pages.length} page${pages.length === 1 ? "" : "s"} selected` : `Use page numbers between 1 and ${max}`}
      </p>
      <div className="modal-actions">
        <button onClick={() => void go()} disabled={!pages || app.busy}>
          Save As…
        </button>
        <button onClick={() => store.setDialog(null)}>Cancel</button>
      </div>
    </Modal>
  );
}

export function SplitDialog() {
  const app = useAppState();
  const max = app.doc?.pages.length ?? 0;
  const [every, setEvery] = useState("1");
  const n = Number(every);
  const valid = Number.isInteger(n) && n >= 1 && n < max;
  const parts = valid ? Math.ceil(max / n) : 0;
  async function go() {
    if (!valid) return;
    if (await splitInto(n)) store.setDialog(null);
  }
  return (
    <Modal title="Split document" onClose={() => store.setDialog(null)}>
      <p className="muted">Write the document as several PDFs into a folder you choose. This document is not changed.</p>
      <label>
        Pages per file
        <input type="number" min={1} max={Math.max(1, max - 1)} value={every} onChange={(e) => setEvery(e.target.value)} onKeyDown={(e) => e.key === "Enter" && void go()} autoFocus />
      </label>
      <p className={valid ? "muted" : "err-note"}>{valid ? `${parts} files` : `Choose 1 to ${max - 1}`}</p>
      <div className="modal-actions">
        <button onClick={() => void go()} disabled={!valid || app.busy}>
          Choose Folder…
        </button>
        <button onClick={() => store.setDialog(null)}>Cancel</button>
      </div>
    </Modal>
  );
}

const FIELDS = ["title", "author", "subject", "keywords"] as const;

/** Document Properties: editable Info fields, file facts and warnings (A-c6, A-c13). */
export function PropertiesDialog() {
  const app = useAppState();
  if (!app.metadataOpen || !app.doc) return null;
  // Remount per opening so the form always starts from the CURRENT values.
  return <PropertiesForm key={app.doc.path} />;
}

function PropertiesForm() {
  const app = useAppState();
  const doc = app.doc!;
  const [form, setForm] = useState<Record<(typeof FIELDS)[number], string>>(() => ({
    title: doc.metadata.title ?? "",
    author: doc.metadata.author ?? "",
    subject: doc.metadata.subject ?? "",
    keywords: doc.metadata.keywords ?? "",
  }));
  const [saved, setSaved] = useState(false);
  const changed = FIELDS.some((k) => (doc.metadata[k] ?? "") !== form[k]);

  async function save() {
    const updates: Record<string, string> = {};
    for (const k of FIELDS) if ((doc.metadata[k] ?? "") !== form[k]) updates[k] = form[k];
    if (await store.updateMetadata(updates)) setSaved(true);
  }
  const warnings = doc.warnings.filter((w) => !/password-protected/.test(w));
  return (
    <Modal title="Document properties" onClose={() => store.setMetadataOpen(false)} wide>
      {FIELDS.map((k) => (
        <label key={k}>
          {k[0].toUpperCase() + k.slice(1)}
          <input value={form[k]} onChange={(e) => (setSaved(false), setForm((f) => ({ ...f, [k]: e.target.value })))} />
        </label>
      ))}
      <dl className="props">
        <dt>File</dt>
        <dd title={doc.path}>{doc.path}</dd>
        <dt>Pages</dt>
        <dd>{doc.pages.length}</dd>
        <dt>Size</dt>
        <dd>{(doc.sourceSize / 1024).toFixed(1)} KB</dd>
        {doc.metadata.creator && (
          <>
            <dt>Created with</dt>
            <dd>{doc.metadata.creator}</dd>
          </>
        )}
        {doc.metadata.producer && (
          <>
            <dt>Producer</dt>
            <dd>{doc.metadata.producer}</dd>
          </>
        )}
        <dt>Security</dt>
        <dd>{doc.encrypted ? "Password-protected (unlocked for this session)" : "None"}</dd>
      </dl>
      {warnings.length > 0 && (
        <div className="warn-box" role="note">
          <b>⚠ Warnings</b>
          <ul>
            {warnings.map((w) => (
              <li key={w}>{w}</li>
            ))}
          </ul>
        </div>
      )}
      <p className="muted">Changes apply to the next Save As (the original file is never modified).</p>
      <div className="modal-actions">
        {saved && <span className="ok-note">Saved ✓</span>}
        <button onClick={() => void save()} disabled={!changed || app.busy}>
          Apply
        </button>
        <button onClick={() => store.setMetadataOpen(false)}>Close</button>
      </div>
    </Modal>
  );
}


// ---------------------------------------------------------------- sizes

const SIZES: { name: string; w: number; h: number }[] = [
  { name: "Letter (8.5 × 11 in)", w: 612, h: 792 },
  { name: "A4 (210 × 297 mm)", w: 595.28, h: 841.89 },
  { name: "A5 (148 × 210 mm)", w: 419.53, h: 595.28 },
  { name: "Legal (8.5 × 14 in)", w: 612, h: 1008 },
  { name: "A3 (297 × 420 mm)", w: 841.89, h: 1190.55 },
];

function SizePicker({ value, onChange, withCurrent }: { value: string; onChange: (v: string) => void; withCurrent?: boolean }) {
  return (
    <label>
      Page size
      <select value={value} onChange={(e) => onChange(e.target.value)} aria-label="Page size">
        {withCurrent && <option value="current">Same as current page</option>}
        {SIZES.map((sz) => (
          <option key={sz.name} value={sz.name}>
            {sz.name}
          </option>
        ))}
      </select>
    </label>
  );
}

const close = () => store.setDialog(null);

export function NewDialog() {
  const [size, setSize] = useState(SIZES[1].name);
  const [landscape, setLandscape] = useState(false);
  async function go() {
    const sz = SIZES.find((x) => x.name === size)!;
    const [w, h] = landscape ? [sz.h, sz.w] : [sz.w, sz.h];
    if (await store.newDocument(w, h)) close();
  }
  return (
    <Modal title="New document" onClose={close}>
      <SizePicker value={size} onChange={setSize} />
      <label className="inline-check">
        <input type="checkbox" checked={landscape} onChange={(e) => setLandscape(e.target.checked)} /> Landscape
      </label>
      <div className="modal-actions">
        <button onClick={() => void go()} autoFocus>
          Create
        </button>
        <button onClick={close}>Cancel</button>
      </div>
    </Modal>
  );
}

export function AddPageDialog() {
  const app = useAppState();
  const [size, setSize] = useState("current");
  const [where, setWhere] = useState<"after" | "end">("after");
  const count = app.doc?.pages.length ?? 0;
  async function go() {
    const after = where === "end" ? count : app.currentPage;
    const sz = SIZES.find((x) => x.name === size);
    if (await store.insertBlank(after, sz?.w, sz?.h)) close();
  }
  return (
    <Modal title="Add blank page" onClose={close}>
      <SizePicker value={size} onChange={setSize} withCurrent />
      <label>
        Position
        <select value={where} onChange={(e) => setWhere(e.target.value as "after" | "end")} aria-label="Position">
          <option value="after">After page {app.currentPage}</option>
          <option value="end">At the end</option>
        </select>
      </label>
      <div className="modal-actions">
        <button onClick={() => void go()} autoFocus>
          Add Page
        </button>
        <button onClick={close}>Cancel</button>
      </div>
    </Modal>
  );
}

const COLORS: Record<string, number[]> = {
  Gray: [0.5, 0.5, 0.5],
  Red: [0.8, 0.1, 0.1],
  Blue: [0.1, 0.25, 0.75],
  Black: [0, 0, 0],
};

export function WatermarkDialog() {
  const app = useAppState();
  const [text, setText] = useState("Confidential");
  const [size, setSize] = useState("64");
  const [placement, setPlacement] = useState("center");
  const [opacity, setOpacity] = useState("50");
  const [color, setColor] = useState("Gray");
  const [scope, setScope] = useState(app.selection.length > 1 ? "selected" : "all");
  async function go() {
    const ok = await store.watermark({
      text,
      size: Number(size),
      placement,
      opacity: Number(opacity) / 100,
      color: COLORS[color],
      pages: scope === "selected" ? store.targets() : [],
    });
    if (ok) close();
  }
  return (
    <Modal title="Watermark" onClose={close}>
      <label>
        Text
        <input value={text} onChange={(e) => setText(e.target.value)} autoFocus aria-label="Watermark text" />
      </label>
      <div className="row2">
        <label>
          Size
          <select value={size} onChange={(e) => setSize(e.target.value)} aria-label="Size">
            {[16, 24, 36, 48, 64, 96].map((n) => (
              <option key={n} value={n}>
                {n} pt
              </option>
            ))}
          </select>
        </label>
        <label>
          Placement
          <select value={placement} onChange={(e) => setPlacement(e.target.value)} aria-label="Placement">
            <option value="center">Center (diagonal)</option>
            <option value="top">Top center</option>
            <option value="tiled">Tiled</option>
          </select>
        </label>
      </div>
      <div className="row2">
        <label>
          Colour
          <select value={color} onChange={(e) => setColor(e.target.value)} aria-label="Colour">
            {Object.keys(COLORS).map((c) => (
              <option key={c}>{c}</option>
            ))}
          </select>
        </label>
        <label>
          Opacity
          <select value={opacity} onChange={(e) => setOpacity(e.target.value)} aria-label="Opacity">
            {[20, 35, 50, 75, 100].map((n) => (
              <option key={n} value={n}>
                {n}%
              </option>
            ))}
          </select>
        </label>
      </div>
      <label>
        Pages
        <select value={scope} onChange={(e) => setScope(e.target.value)} aria-label="Pages">
          <option value="all">All pages</option>
          <option value="selected">{app.selection.length > 1 ? `${app.selection.length} selected pages` : `Page ${app.currentPage}`}</option>
        </select>
      </label>
      <div className="modal-actions">
        <button onClick={() => void go()} disabled={!text.trim() || app.busy}>
          Apply
        </button>
        <button onClick={close}>Cancel</button>
      </div>
    </Modal>
  );
}

export function EncryptDialog() {
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const [owner, setOwner] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const ok = pw.length > 0 && pw === pw2;
  async function go() {
    const doc = getState().doc;
    if (!doc || !ok) return;
    const base = docName(doc).replace(/\.pdf$/i, "");
    const dir = isUntitled(doc) ? "" : doc.path.slice(0, doc.path.lastIndexOf("/") + 1);
    const target = await saveDialog({ defaultPath: `${dir}${base}-protected.pdf`, filters: [{ name: "PDF", extensions: ["pdf"] }] });
    if (typeof target !== "string") return;
    try {
      const r = await cmdSaveEncrypted(target, pw, owner);
      store.notify(`Saved a password-protected copy → ${r.published_path}`);
      close();
    } catch (e) {
      setErr((e as Error).message ?? String(e));
    }
  }
  return (
    <Modal title="Save As Encrypted" onClose={close}>
      <p className="muted">Saves a copy that needs a password to open (AES-256). This document is not changed.</p>
      <label>
        Password
        <input type="password" value={pw} onChange={(e) => setPw(e.target.value)} autoFocus aria-label="Password" />
      </label>
      <label>
        Confirm password
        <input type="password" value={pw2} onChange={(e) => setPw2(e.target.value)} aria-label="Confirm password" onKeyDown={(e) => e.key === "Enter" && void go()} />
      </label>
      <label>
        Owner password (optional — for changing permissions)
        <input type="password" value={owner} onChange={(e) => setOwner(e.target.value)} aria-label="Owner password" />
      </label>
      {pw2 && pw !== pw2 && <p className="err-note">Passwords don’t match</p>}
      {err && <p className="err-note">{err}</p>}
      <div className="modal-actions">
        <button onClick={() => void go()} disabled={!ok}>
          Save As…
        </button>
        <button onClick={close}>Cancel</button>
      </div>
    </Modal>
  );
}

export function AttachmentsDialog() {
  const [list, setList] = useState<AttachmentInfo[] | null>(null);
  useEffect(() => {
    cmdListAttachments().then(setList, () => setList([]));
  }, []);
  async function saveOne(a: AttachmentInfo) {
    const target = await saveDialog({ defaultPath: a.name });
    if (typeof target !== "string") return;
    try {
      await cmdSaveAttachment(a.index, target);
      store.notify(`Saved ${a.name}`);
    } catch (e) {
      store.notify(`Could not save: ${(e as Error).message ?? e}`);
    }
  }
  return (
    <Modal title="Attachments" onClose={close} wide>
      {list === null ? (
        <p className="muted">Reading…</p>
      ) : list.length === 0 ? (
        <p className="muted">This document has no embedded files.</p>
      ) : (
        <table className="license-table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Size</th>
              <th>Type</th>
              <th>Description</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {list.map((a) => (
              <tr key={a.index}>
                <td>{a.name}</td>
                <td>{a.size ? `${(a.size / 1024).toFixed(1)} KB` : "—"}</td>
                <td>{a.mime || "—"}</td>
                <td>{a.description || "—"}</td>
                <td>
                  <button onClick={() => void saveOne(a)}>Save…</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div className="modal-actions">
        <button onClick={close}>Close</button>
      </div>
    </Modal>
  );
}

export function ExportImagesDialog() {
  const app = useAppState();
  const [fmt, setFmt] = useState<"png" | "jpg">("png");
  const [dpi, setDpi] = useState("150");
  const [range, setRange] = useState("all");
  const [progress, setProgress] = useState<string | null>(null);
  const count = app.doc?.pages.length ?? 0;
  async function go() {
    const s = getState();
    if (!s.session || !s.doc) return;
    const pages = range === "all" ? s.doc.pages.map((p) => p.page_no) : store.targets();
    const base = docName(s.doc).replace(/\.pdf$/i, "");
    const dir = isUntitled(s.doc) ? "" : s.doc.path.slice(0, s.doc.path.lastIndexOf("/") + 1);
    const target = await saveDialog({ defaultPath: `${dir}${base}.${fmt}`, filters: [{ name: fmt.toUpperCase(), extensions: [fmt] }] });
    if (typeof target !== "string") return;
    const stem = target.replace(/\.(png|jpe?g)$/i, "");
    for (const [i, n] of pages.entries()) {
      setProgress(`Page ${i + 1} of ${pages.length}…`);
      const rot = s.doc.pages.find((p) => p.page_no === n)?.rotation ?? 0;
      const bytes = await pageImage(s.session, n, rot, Number(dpi), fmt === "png" ? "image/png" : "image/jpeg");
      const name = pages.length === 1 ? `${stem}.${fmt}` : `${stem}-${n}.${fmt}`;
      try {
        await cmdWriteFile(name, toBase64(bytes));
      } catch (e) {
        setProgress(null);
        store.notify(`Export failed: ${(e as Error).message ?? e}`);
        return;
      }
    }
    store.notify(`Exported ${pages.length} image${pages.length === 1 ? "" : "s"}`);
    close();
  }
  return (
    <Modal title="Export pages as images" onClose={close}>
      <div className="row2">
        <label>
          Format
          <select value={fmt} onChange={(e) => setFmt(e.target.value as "png" | "jpg")} aria-label="Format">
            <option value="png">PNG</option>
            <option value="jpg">JPEG</option>
          </select>
        </label>
        <label>
          Resolution
          <select value={dpi} onChange={(e) => setDpi(e.target.value)} aria-label="Resolution">
            {[72, 150, 300].map((d) => (
              <option key={d} value={d}>
                {d} dpi
              </option>
            ))}
          </select>
        </label>
      </div>
      <label>
        Pages
        <select value={range} onChange={(e) => setRange(e.target.value)} aria-label="Pages">
          <option value="all">All {count} pages</option>
          <option value="sel">{app.selection.length > 1 ? `${app.selection.length} selected pages` : `Page ${app.currentPage}`}</option>
        </select>
      </label>
      {progress && <p className="muted">{progress}</p>}
      <div className="modal-actions">
        <button onClick={() => void go()} disabled={!!progress}>
          Export…
        </button>
        <button onClick={close}>Cancel</button>
      </div>
    </Modal>
  );
}

/** Print + Print Preview: paper, orientation, range; the sheet preview uses the
 *  same fit maths as the printout. */
export function PrintDialog() {
  const app = useAppState();
  const [paper, setPaper] = useState("A4");
  const [landscape, setLandscape] = useState(false);
  const [range, setRange] = useState("all");
  const [custom, setCustom] = useState("");
  const [at, setAt] = useState(0);
  const [busy, setBusy] = useState(false);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const doc = app.doc!;
  const count = doc.pages.length;
  const pages =
    range === "all" ? doc.pages.map((p) => p.page_no) : range === "current" ? [app.currentPage] : parsePages(custom, count) ?? [];
  const pp = PAPERS.find((p) => p.name === paper)!;
  const sheet = landscape ? { w: pp.h, h: pp.w } : { w: pp.w, h: pp.h };
  const shown = pages[Math.min(at, Math.max(0, pages.length - 1))];
  const scale = 300 / Math.max(sheet.w, sheet.h);

  useEffect(() => {
    const s = getState().session;
    const c = canvasRef.current;
    if (!s || !c || !shown) return;
    let alive = true;
    const meta = doc.pages.find((p) => p.page_no === shown)!;
    renderPage(s, shown, 1, meta.rotation).then((r) => {
      if (!alive) return;
      const f = fitOnSheet({ w: r.sizePt.width, h: r.sizePt.height }, sheet);
      c.width = Math.round(sheet.w * scale);
      c.height = Math.round(sheet.h * scale);
      const ctx = c.getContext("2d")!;
      ctx.fillStyle = "#fff";
      ctx.fillRect(0, 0, c.width, c.height);
      ctx.setLineDash([4, 3]);
      ctx.strokeStyle = "#9aa";
      ctx.strokeRect(18 * scale, 18 * scale, (sheet.w - 36) * scale, (sheet.h - 36) * scale);
      ctx.drawImage(r.image, f.x * scale, f.y * scale, f.w * scale, f.h * scale);
    });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shown, paper, landscape, app.session]);

  async function go() {
    const s = getState().session;
    if (!s || !pages.length) return;
    setBusy(true);
    try {
      await printDocument(s, new Map(doc.pages.map((p) => [p.page_no, p.rotation])), { pages, paper: pp, landscape });
      store.notify(`Sent ${pages.length} page${pages.length === 1 ? "" : "s"} to the print panel`);
      close();
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title="Print" onClose={close} wide>
      <div className="print-layout">
        <div className="print-preview">
          <canvas ref={canvasRef} aria-label="Print preview" />
          <div className="print-nav">
            <button onClick={() => setAt((i) => Math.max(0, i - 1))} disabled={at <= 0} aria-label="Previous sheet">
              ‹
            </button>
            <span>
              Sheet {Math.min(at + 1, pages.length)} of {pages.length}
            </span>
            <button onClick={() => setAt((i) => Math.min(pages.length - 1, i + 1))} disabled={at >= pages.length - 1} aria-label="Next sheet">
              ›
            </button>
          </div>
        </div>
        <div className="print-options">
          <label>
            Paper
            <select value={paper} onChange={(e) => setPaper(e.target.value)} aria-label="Paper">
              {PAPERS.map((p) => (
                <option key={p.name}>{p.name}</option>
              ))}
            </select>
          </label>
          <label>
            Orientation
            <select value={landscape ? "l" : "p"} onChange={(e) => setLandscape(e.target.value === "l")} aria-label="Orientation">
              <option value="p">Portrait</option>
              <option value="l">Landscape</option>
            </select>
          </label>
          <label>
            Pages
            <select value={range} onChange={(e) => (setRange(e.target.value), setAt(0))} aria-label="Print range">
              <option value="all">All ({count})</option>
              <option value="current">Current page ({app.currentPage})</option>
              <option value="custom">Pages…</option>
            </select>
          </label>
          {range === "custom" && (
            <label>
              Page range (e.g. 1-3, 5)
              <input value={custom} onChange={(e) => (setCustom(e.target.value), setAt(0))} aria-label="Custom pages" autoFocus />
            </label>
          )}
          <p className="muted">The macOS print panel opens next — copies, printer, and “Save as PDF” are there.</p>
        </div>
      </div>
      <div className="modal-actions">
        <button onClick={() => void go()} disabled={!pages.length || busy}>
          {busy ? "Preparing…" : "Print…"}
        </button>
        <button onClick={close}>Cancel</button>
      </div>
    </Modal>
  );
}

export function HelpDialog() {
  return (
    <Modal title="Stitchup — User Guide" onClose={close} wide>
      <div className="help-body">
        <h3>Opening and saving</h3>
        <p>Open a PDF with ⌘O, by dropping it on the window or the Dock icon, or from Finder. ⌘S saves in place (the file is replaced atomically, never half-written); ⌘⇧S saves under a new name. The Save button turns red while there are unsaved changes.</p>
        <h3>Ribbon</h3>
        <p><b>Document</b> — new, open, save, properties, import, exports (text, CSV, images, RTF, DOCX, ODT), print. <b>Pages</b> — rotate, delete, add, extract, split, auto-crop. <b>Content</b> — Edit Text. <b>Zoom</b>, <b>Navigate</b> — view controls. <b>Security</b> — watermark and password protection.</p>
        <h3>Editing text</h3>
        <p>Double-click any line (or Content ▸ Edit Text, then click it). It is edited in place, in the document’s own font when it has the needed letters. A bar above the line shows its font and size: change the font, bold / italic, size or colour there, or with Content ▸ Fore and Font. Enter or clicking away saves; Tab moves to the next line; Esc cancels; clearing the text deletes the line.</p>
        <h3>Pages sidebar</h3>
        <p>Drag thumbnails to reorder. ⌘-click and Shift-click select several pages; Rotate, Delete, Extract and Watermark then act on the selection. F8 shows or hides the sidebar.</p>
        <h3>Keyboard</h3>
        <table className="license-table">
          <tbody>
            {[
              ["⌘N / ⌘O / ⌘S / ⌘⇧S", "New / Open / Save / Save As"],
              ["⌘Z / ⌘⇧Z", "Undo / Redo"],
              ["⌘F, F3 / ⇧F3", "Find, next / previous match"],
              ["⌘R / ⌘⇧R", "Rotate right / left"],
              ["⌫", "Delete selected page(s)"],
              ["⌘A", "Select all pages"],
              ["⌘= / ⌘− / ⌘1 / ⌘2 / ⌘0", "Zoom in / out / 100% / fit width / fit page"],
              ["⌘↑ ⌘↓, PgUp PgDn", "Previous / next page"],
              ["F5 / F8", "Two-page spread / sidebar"],
              ["⌘P / ⌘⇧P", "Print / print preview"],
              ["⌘D", "Dark mode"],
              ["F1", "This guide"],
            ].map(([k, v]) => (
              <tr key={k}>
                <td>
                  <kbd>{k}</kbd>
                </td>
                <td>{v}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="modal-actions">
        <button onClick={close} autoFocus>
          Close
        </button>
      </div>
    </Modal>
  );
}


// ------------------------------------------------- annotation text inputs

/** Text for a just-placed Note / Text Box / Link (and placement of a signature). */
export function AnnotInputDialog() {
  const app = useAppState();
  const p = app.pendingInput!;
  const [text, setText] = useState("");
  const [size, setSize] = useState("12");
  const done = () => store.setPendingInput(null);

  // A signature box was just drawn: sign straight away (details were entered first).
  useEffect(() => {
    if (p.kind !== "sign") return;
    done();
    void signNow({ ...(pendingSign ?? { name: "" }), page: p.page, rect: p.rect });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  if (p.kind === "sign") return null;

  async function ok() {
    const t = text.trim();
    if (p.kind === "link") {
      if (!t) return;
      const page = /^#?\d+$/.test(t) ? Number(t.replace("#", "")) : null;
      if (page !== null && (page < 1 || page > (app.doc?.pages.length ?? 0))) return store.notify(`No page ${page}`);
      await store.annotate(p.page, page !== null ? { kind: "link", rect: p.rect, page } : { kind: "link", rect: p.rect, uri: t });
    } else if (p.kind === "note") {
      await store.annotate(p.page, { kind: "note", at: [p.rect[0], p.rect[1]], text: t, color: [1, 0.85, 0.2] });
    } else {
      if (!t) return;
      await store.annotate(p.page, { kind: "text_box", rect: p.rect, text: t, size: Number(size), color: rgb(annotColor(app, "other")) });
    }
    done();
  }
  const title = p.kind === "note" ? "Sticky note" : p.kind === "link" ? "Link" : "Text box";
  return (
    <Modal title={title} onClose={done}>
      {p.kind === "link" ? (
        <label>
          Web address, or a page number to jump to
          <input value={text} onChange={(e) => setText(e.target.value)} placeholder="example.com or 3" autoFocus aria-label="Link target" onKeyDown={(e) => e.key === "Enter" && void ok()} />
        </label>
      ) : (
        <label>
          {p.kind === "note" ? "Comment" : "Text"}
          <textarea className="edit-input" rows={4} value={text} onChange={(e) => setText(e.target.value)} autoFocus aria-label={p.kind === "note" ? "Comment" : "Text"} />
        </label>
      )}
      {p.kind === "textbox" && (
        <label>
          Size
          <select value={size} onChange={(e) => setSize(e.target.value)} aria-label="Text size">
            {[8, 10, 12, 14, 18, 24, 36].map((n) => (
              <option key={n} value={n}>
                {n} pt
              </option>
            ))}
          </select>
        </label>
      )}
      <div className="modal-actions">
        <button onClick={() => void ok()} disabled={app.busy || (p.kind !== "note" && !text.trim())}>
          Add
        </button>
        <button onClick={done}>Cancel</button>
      </div>
    </Modal>
  );
}

// ------------------------------------------------------------- signature

interface SignDetails {
  name: string;
  reason?: string;
  location?: string;
  contact?: string;
  page?: number;
  rect?: number[];
}

/** Details entered in the Sign dialog while the user draws the box. */
let pendingSign: SignDetails | null = null;

async function signNow(spec: SignDetails) {
  const doc = getState().doc;
  store.setTool(null);
  if (!doc) return;
  const base = docName(doc).replace(/\.pdf$/i, "");
  const dir = isUntitled(doc) ? "" : doc.path.slice(0, doc.path.lastIndexOf("/") + 1);
  const target = await saveDialog({ defaultPath: `${dir}${base}-signed.pdf`, filters: [{ name: "PDF", extensions: ["pdf"] }] });
  if (typeof target !== "string") return;
  try {
    const r = await cmdSign(target, spec);
    store.notify(`Signed by ${spec.name} → ${r.published_path}`);
  } catch (e) {
    store.notify(`Signing failed: ${(e as Error).message ?? e}`);
  }
}

const SIGNER_KEY = "stitchup.signer";

export function SignDialog() {
  const saved = (() => {
    try {
      return JSON.parse(localStorage.getItem(SIGNER_KEY) ?? "{}") as Partial<SignDetails>;
    } catch {
      return {};
    }
  })();
  const [name, setName] = useState(saved.name ?? "");
  const [reason, setReason] = useState(saved.reason ?? "I approve this document");
  const [location, setLocation] = useState(saved.location ?? "");
  const [visible, setVisible] = useState(true);
  const app = useAppState();
  async function go() {
    const details: SignDetails = { name: name.trim(), reason: reason.trim(), location: location.trim() };
    try {
      localStorage.setItem(SIGNER_KEY, JSON.stringify(details));
    } catch {
      /* not remembered */
    }
    close();
    if (visible) {
      pendingSign = details;
      store.setTool("sign");
      store.notify("Drag the signature box on the page (click for a default size, Esc to cancel)");
    } else {
      await signNow({ ...details, page: app.currentPage });
    }
  }
  return (
    <Modal title="Sign document" onClose={close}>
      <p className="muted">
        Adds a digital signature (RSA-2048, SHA-256) and saves a signed copy; this document stays as it is. Stitchup creates a
        self-signed certificate, so readers will show the signature as valid but from an unverified identity.
      </p>
      <label>
        Your name
        <input value={name} onChange={(e) => setName(e.target.value)} autoFocus aria-label="Signer name" />
      </label>
      <label>
        Reason
        <input value={reason} onChange={(e) => setReason(e.target.value)} aria-label="Reason" />
      </label>
      <label>
        Location (optional)
        <input value={location} onChange={(e) => setLocation(e.target.value)} aria-label="Location" />
      </label>
      <label className="inline-check">
        <input type="checkbox" checked={visible} onChange={(e) => setVisible(e.target.checked)} aria-label="Visible signature" /> Show a signature box on the page
      </label>
      <div className="modal-actions">
        <button onClick={() => void go()} disabled={!name.trim()}>
          {visible ? "Place Signature…" : "Sign & Save As…"}
        </button>
        <button onClick={close}>Cancel</button>
      </div>
    </Modal>
  );
}

// ------------------------------------------------------- recolour / font

const PRESETS = ["#000000", "#d0021b", "#1d5fd0", "#1a7f37", "#7c4dbe", "#f5a623", "#666666", "#ffffff"];

export function RecolorDialog() {
  const app = useAppState();
  const [color, setColor] = useState(app.foreColor);
  return (
    <Modal title="Recolour text" onClose={close}>
      <div className="swatch-row">
        {PRESETS.map((c) => (
          <button key={c} className={`swatch-btn${c === color ? " on" : ""}`} style={{ background: c }} onClick={() => setColor(c)} aria-label={c} />
        ))}
        <input type="color" value={color} onChange={(e) => setColor(e.target.value)} aria-label="Custom colour" />
      </div>
      <div className="modal-actions">
        <button
          onClick={async () => {
            if (await store.objectOp({ op: "recolor", color: rgb(color) })) close();
          }}>
          Apply
        </button>
        <button onClick={close}>Cancel</button>
      </div>
    </Modal>
  );
}

export function FontDialog() {
  const faces = useSystemFonts();
  const [family, setFamily] = useState<FontChoice>("Helvetica");
  const [bold, setBold] = useState(false);
  const [italic, setItalic] = useState(false);
  const [size, setSize] = useState("");
  return (
    <Modal title="Font and size" onClose={close}>
      <label>
        Font
        <FontPicker value={family} onPick={setFamily} />
      </label>
      <label className="inline-check">
        <input type="checkbox" checked={bold} onChange={(e) => setBold(e.target.checked)} /> Bold
      </label>
      <label className="inline-check">
        <input type="checkbox" checked={italic} onChange={(e) => setItalic(e.target.checked)} /> Italic
      </label>
      <label>
        Size (pt, empty = keep)
        <input value={size} onChange={(e) => setSize(e.target.value)} inputMode="decimal" aria-label="Font size" />
      </label>
      <p className="muted">Installed fonts are embedded (only the letters used), so the PDF looks the same on every computer.</p>
      <div className="modal-actions">
        <button
          onClick={async () => {
            const n = Number(size);
            if (await store.objectOp({ op: "font", family: fontName(faces, family, bold, italic), size: size.trim() && n > 0 ? n : undefined })) close();
          }}>
          Apply
        </button>
        <button onClick={close}>Cancel</button>
      </div>
    </Modal>
  );
}


// ---------------------------------------------------------------- settings

export function SettingsDialog() {
  const [st, setSt] = useState<Settings>(loadSettings);
  const [scheme, setScheme] = useState<ThemeName>(currentTheme());
  function save() {
    saveSettings(st);
    void applyTheme(scheme);
    store.notify("Settings saved");
    close();
  }
  return (
    <Modal title="Settings" onClose={close}>
      <label>
        Crash-recovery snapshots
        <select aria-label="Crash-recovery snapshots" value={st.recoverySeconds} onChange={(e) => setSt({ ...st, recoverySeconds: Number(e.target.value) })}>
          <option value={0}>Off</option>
          <option value={30}>Every 30 seconds</option>
          <option value={60}>Every minute</option>
          <option value={300}>Every 5 minutes</option>
        </select>
      </label>
      <p className="muted">
        Unsaved documents are copied into Stitchup’s private folder (never over your files) and offered back if Stitchup quits
        unexpectedly. Your own files are only written when you save.
      </p>
      <label>
        When a document opens
        <select aria-label="Opening zoom" value={st.openZoom} onChange={(e) => setSt({ ...st, openZoom: e.target.value as Settings["openZoom"] })}>
          <option value="fit-width">Fit width</option>
          <option value="fit-page">Fit page</option>
          <option value="actual">100%</option>
        </select>
      </label>
      <label className="inline-check">
        <input type="checkbox" checked={st.sidebarOnOpen} onChange={(e) => setSt({ ...st, sidebarOnOpen: e.target.checked })} /> Show the page sidebar
      </label>
      <label>
        Colour scheme
        <select aria-label="Colour scheme" value={scheme} onChange={(e) => setScheme(e.target.value as ThemeName)}>
          {THEMES.map((t) => (
            <option key={t.name} value={t.name}>
              {t.label}
            </option>
          ))}
        </select>
      </label>
      <div className="modal-actions">
        <button onClick={save}>Save</button>
        <button onClick={close}>Cancel</button>
      </div>
    </Modal>
  );
}

/** Offered at launch when a previous Stitchup left unsaved snapshots behind. */
export function RecoveryDialog({ entries, onDone }: { entries: RecoveryEntry[]; onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  async function restoreAll() {
    setBusy(true);
    for (const e of entries) await store.restoreRecovered(e.file, e.name);
    await cmdDiscardRecovery().catch(() => {});
    onDone();
  }
  async function discard() {
    await cmdDiscardRecovery().catch(() => {});
    onDone();
  }
  return (
    <Modal title="Recover unsaved documents?" onClose={onDone}>
      <p>Stitchup didn’t close properly last time. These documents had unsaved changes:</p>
      <ul className="recovery-list">
        {entries.map((e) => (
          <li key={e.file}>
            <b>{e.name}</b>
            <span className="muted"> — {new Date(e.saved_at * 1000).toLocaleString()}</span>
          </li>
        ))}
      </ul>
      <p className="muted">Recovered documents open as new, unsaved documents; use Save As to keep them.</p>
      <div className="modal-actions">
        <button onClick={() => void restoreAll()} disabled={busy} autoFocus>
          Recover
        </button>
        <button onClick={() => void discard()} disabled={busy}>
          Discard
        </button>
      </div>
    </Modal>
  );
}
