import { useEffect, useRef, useState, type ReactNode } from "react";
import { useAppState, store } from "../../store";
import {
  bind,
  renderPage,
  unbind,
  getTextRuns,
  RenderSession,
  RenderedPage,
} from "../../adapters/renderAdapter";
import type { TextRun } from "../../types";

interface EditPick {
  pageNo: number;
  run: TextRun;
}

function EditOverlay({
  session,
  pageNo,
  zoom,
  onPick,
}: {
  session: RenderSession;
  pageNo: number;
  zoom: number;
  onPick: (pick: EditPick) => void;
}) {
  const [runs, setRuns] = useState<TextRun[]>([]);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    getTextRuns(session, pageNo)
      .then((r) => alive && setRuns(r))
      .catch((e) => alive && setErr(String(e)));
    return () => {
      alive = false;
    };
  }, [session, pageNo]);

  if (err) return null;
  return (
    <div className="edit-overlay edit-overlay-on">
      {runs.map((run, i) => (
        <span
          key={i}
          className="text-run-edit"
          style={{
            left: run.rect.x * zoom,
            top: run.rect.y * zoom,
            width: run.rect.width * zoom,
            height: run.rect.height * zoom,
          }}
          title={run.str}
          onClick={() => onPick({ pageNo, run })}
        />
      ))}
    </div>
  );
}

/**
 * A transparent text layer over the rendered page so the user can select and
 * copy logical text (A-c10). Runs come from the text layer via the adapter;
 * selection is handled natively by the webview, so the OS clipboard receives
 * the Unicode characters, not glyph ids. Only shown on un-rotated pages: the
 * text-run geometry is extracted axis-aligned (a documented Stage 1 limit).
 */
function TextSelectLayer({
  session,
  pageNo,
  zoom,
  rotation,
  enabled,
}: {
  session: RenderSession;
  pageNo: number;
  zoom: number;
  rotation: number;
  enabled: boolean;
}) {
  const [runs, setRuns] = useState<TextRun[]>([]);
  const upright = rotation % 360 === 0;

  useEffect(() => {
    if (!upright || !enabled) return;
    let alive = true;
    getTextRuns(session, pageNo)
      .then((r) => {
        if (alive) setRuns(r);
      })
      .catch(() => {
        if (alive) setRuns([]);
      });
    return () => {
      alive = false;
    };
  }, [session, pageNo, upright, enabled]);

  if (!upright) return null;

  return (
    <div className="text-layer">
      {runs.map((run, i) => (
        <span
          key={i}
          className="text-run"
          style={{
            left: run.rect.x * zoom,
            top: run.rect.y * zoom,
            width: run.rect.width * zoom,
            fontSize: run.rect.height * zoom,
          }}>
          {run.str}
        </span>
      ))}
    </div>
  );
}

function EditTextModal({
  pageNo,
  run,
  pageHeightPt,
  onCancel,
}: {
  pageNo: number;
  run: TextRun;
  pageHeightPt: number;
  onCancel: () => void;
}) {
  const [text, setText] = useState(run.str);
  const [busy, setBusy] = useState(false);

  async function apply() {
    const cleaned = text.trim();
    if (!cleaned) return;
    setBusy(true);
    // rect is top-left origin (viewport pts); convert to PDF user space.
    const y = pageHeightPt - run.rect.y - run.rect.height;
    const ok = await store.editText({
      page: pageNo,
      x: run.rect.x,
      y,
      width: run.rect.width,
      height: run.rect.height,
      font_size: Math.max(4, run.rect.height),
      new_text: cleaned,
    });
    setBusy(false);
    if (ok) onCancel();
  }

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal" role="dialog" aria-label="Replace text" onClick={(e) => e.stopPropagation()}>
        <h2>Replace text (page {pageNo})</h2>
        <label>
          Replacement text
          <textarea
            className="edit-input"
            rows={3}
            value={text}
            autoFocus
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => e.key === "Escape" && onCancel()}
          />
        </label>
        <p className="muted edit-note">
          Rendered over the original with a white patch + Helvetica at font size {Math.max(4, run.rect.height).toFixed(1)} pt.
        </p>
        <div className="modal-actions">
          <button onClick={onCancel}>Cancel</button>
          <button onClick={() => void apply()} disabled={!text.trim() || busy}>
            {busy ? "Applying…" : "Apply"}
          </button>
        </div>
      </div>
    </div>
  );
}

function PageView({
  session,
  pageNo,
  editMode,
  onPick,
}: {
  session: RenderSession;
  pageNo: number;
  editMode: boolean;
  onPick: (pick: EditPick) => void;
}) {
  const app = useAppState();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const [rendered, setRendered] = useState<RenderedPage | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);

  const docMeta = session.doc.pages.find((p) => p.page_no === pageNo);
  const rotation = docMeta?.rotation ?? 0;

  // Load the selectable text layer only once the page is near the viewport, so
  // a 1000-page document (F-11) doesn't fetch every page's text up front.
  useEffect(() => {
    if (visible) return;
    const el = rootRef.current;
    if (!el || typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setVisible(true);
          io.disconnect();
        }
      },
      { rootMargin: "300px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [visible]);

  useEffect(() => {
    let alive = true;
    const scale = app.zoom;
    renderPage(session, pageNo, scale, rotation)
      .then((r) => {
        if (alive) setRendered(r);
      })
      .catch((e) => alive && setErr(String(e)));
    return () => {
      alive = false;
    };
  }, [session, pageNo, app.zoom, app.zoomMode, rotation]);

  // Paint only after React has committed the final canvas backing size: setting
  // `width`/`height` on a <canvas> clears it, so drawing during the render
  // promise (before the size is applied) would wipe the page.
  useEffect(() => {
    const c = canvasRef.current;
    if (!rendered || !c) return;
    const ctx = c.getContext("2d");
    ctx?.drawImage(rendered.image, 0, 0);
  }, [rendered]);

  return (
    <div
      ref={rootRef}
      className={`page-view${err ? " has-error" : ""}`}
      style={{ width: rendered?.sizePx.width, height: rendered?.sizePx.height }}
    >
      <canvas ref={canvasRef} width={rendered?.image.width} height={rendered?.image.height} />
      {err && <div className="page-error">{err}</div>}
      {editMode && !err && <EditOverlay session={session} pageNo={pageNo} zoom={app.zoom} onPick={onPick} />}
      {!editMode && !err && (
        <TextSelectLayer
          session={session}
          pageNo={pageNo}
          zoom={app.zoom}
          rotation={rotation}
          enabled={visible}
        />
      )}
    </div>
  );
}

export function Viewer() {
  const app = useAppState();
  const [session, setSession] = useState<RenderSession | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [editTarget, setEditTarget] = useState<EditPick | null>(null);

  // Mirror the document selection into the store so Copy can enable/disable and
  // report a count (A-c10).
  useEffect(() => {
    const onSelectionChange = () => {
      store.setSelection(document.getSelection()?.toString() ?? "");
    };
    document.addEventListener("selectionchange", onSelectionChange);
    return () => document.removeEventListener("selectionchange", onSelectionChange);
  }, []);

  useEffect(() => {
    if (!app.doc) {
      setSession(null);
      return;
    }
    let alive = true;
    void bind(app.doc)
      .then((s) => {
        if (alive) setSession(s);
      })
      .catch((e) => alive && setErr(String(e)));
    return () => {
      alive = false;
      if (session) void unbind(session);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [app.doc?.bytesB64]);

  if (!app.doc) {
    return <div className="empty-viewer">No document open — use Open…</div>;
  }
  if (err) {
    return <div className="viewer-error">Renderer failed: {err}</div>;
  }
  if (!session) {
    return <div className="empty-viewer">Parsing document…</div>;
  }

  const overlay = app.editMode && (
    <div className="viewer-hint">
      Click a highlighted line to replace its text · Esc to cancel
    </div>
  );

  const pageCount = app.doc.pages.length;
  let body: ReactNode;
  if (app.viewMode === "single") {
    body = <PageView session={session} pageNo={app.currentPage} editMode={app.editMode} onPick={setEditTarget} />;
  } else if (app.viewMode === "spread") {
    const pairs: number[][] = [];
    for (let i = 1; i <= pageCount; i += 2) {
      pairs.push(i < pageCount ? [i, i + 1] : [i]);
    }
    body = (
      <>
        {pairs.map((pair) => (
          <div className="page-spread" key={pair.join("-")}>
            {pair.map((p) => (
              <PageView key={p} session={session} pageNo={p} editMode={app.editMode} onPick={setEditTarget} />
            ))}
          </div>
        ))}
      </>
    );
  } else {
    body = (
      <>
        {app.doc.pages.map((p) => (
          <PageView key={p.page_no} session={session} pageNo={p.page_no} editMode={app.editMode} onPick={setEditTarget} />
        ))}
      </>
    );
  }

  const pageMeta = editTarget
    ? app.doc.pages.find((p) => p.page_no === editTarget.pageNo)
    : undefined;

  return (
    <div className="viewer-scroll">
      {app.editMode && overlay}
      {body}
      {editTarget && (
        <EditTextModal
          pageNo={editTarget.pageNo}
          run={editTarget.run}
          pageHeightPt={pageMeta?.height_pt ?? 792}
          onCancel={() => {
            setEditTarget(null);
            store.setEditMode(false);
          }}
        />
      )}
    </div>
  );
}