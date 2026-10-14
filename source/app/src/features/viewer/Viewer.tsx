import { useEffect, useRef, useState, type ReactNode } from "react";
import { useAppState, store } from "../../store";
import {
  bind,
  unbind,
  renderPage,
  getTextRuns,
  RenderSession,
  RenderedPage,
} from "../../adapters/renderAdapter";
import type { TextRun } from "../../types";

/**
 * A transparent text layer over the rendered page so the user can select and
 * copy logical text (A-c10) AND double-click a line to edit it live. Runs come
 * from the text layer via the adapter; selection is handled natively by the
 * webview, so the OS clipboard receives the Unicode characters, not glyph ids.
 * Only shown on un-rotated pages: the text-run geometry is extracted
 * axis-aligned (a documented Stage 1 limit).
 */
function TextLayer({
  session,
  pageNo,
  zoom,
  rotation,
  editing,
  onStartEdit,
}: {
  session: RenderSession;
  pageNo: number;
  zoom: number;
  rotation: number;
  editing: boolean;
  onStartEdit: (run: TextRun, caretX: number) => void;
}) {
  const [runs, setRuns] = useState<TextRun[]>([]);
  const upright = rotation % 360 === 0;

  useEffect(() => {
    if (!upright) return;
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
  }, [session, pageNo, upright]);

  if (!upright) return null;

  return (
    <div className="text-layer" aria-hidden={editing}>
      {runs.map((run, i) => (
        <span
          key={i}
          className="text-run"
          style={{
            left: run.rect.x * zoom,
            top: run.rect.y * zoom,
            width: run.rect.width * zoom,
            height: run.rect.height * zoom,
            fontSize: run.rect.height * zoom,
          }}
          title={run.str}
          onDoubleClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            onStartEdit(run, e.clientX - e.currentTarget.getBoundingClientRect().left);
          }}>
          {run.str}
        </span>
      ))}
    </div>
  );
}

/**
 * Inline live editor placed over a text run. The webview caret lands where the
 * user double-clicked (approximated by fraction of the run width). Enter or
 * blur commits the replacement (white-patch + Helvetica via the core op);
 * Escape cancels.
 */
function InlineEditor({
  run,
  zoom,
  caretX,
  onCommit,
  onCancel,
}: {
  run: TextRun;
  zoom: number;
  caretX: number;
  onCommit: (text: string) => void;
  onCancel: () => void;
}) {
  const [text, setText] = useState(run.str);
  const ref = useRef<HTMLTextAreaElement | null>(null);
  const committed = useRef(false);

  useEffect(() => {
    const ta = ref.current;
    if (!ta) return;
    ta.focus();
    const w = Math.max(run.rect.width * zoom, 1);
    const frac = Math.min(1, Math.max(0, caretX / w));
    const idx = Math.round(frac * run.str.length);
    ta.setSelectionRange(idx, idx);
  }, [run, zoom, caretX]);

  function commit() {
    if (committed.current) return;
    committed.current = true;
    onCommit(text);
  }

  return (
    <textarea
      ref={ref}
      className="inline-editor"
      value={text}
      spellCheck={false}
      style={{
        left: run.rect.x * zoom,
        top: run.rect.y * zoom,
        width: Math.max(run.rect.width * zoom, 28),
        height: Math.max(run.rect.height * zoom * 1.35, 14),
        fontSize: Math.max(run.rect.height * zoom, 6),
        lineHeight: 1,
      }}
      onChange={(e) => setText(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          e.stopPropagation();
          commit();
        } else if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          committed.current = true;
          onCancel();
        }
      }}
      onBlur={commit}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
    />
  );
}

function PageView({ session, pageNo }: { session: RenderSession; pageNo: number }) {
  const app = useAppState();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const [rendered, setRendered] = useState<RenderedPage | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);
  const [editing, setEditing] = useState<{ run: TextRun; caretX: number } | null>(null);

  const docMeta = session.doc.pages.find((p) => p.page_no === pageNo);
  const rotation = docMeta?.rotation ?? 0;
  const canEdit = !session.doc.passwordRequired && !session.doc.encrypted;

  // Load the selectable/editable text layer only once the page is near the
  // viewport, so a 1000-page document (F-11) doesn't fetch every page up front.
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

  async function commitEdit(run: TextRun, text: string) {
    setEditing(null);
    const next = text.trim();
    if (next === run.str.trim() || next.length === 0) return;
    const heightPt = docMeta?.height_pt ?? 792;
    // rect is top-left origin (viewport pts); convert to PDF user space.
    const y = heightPt - run.rect.y - run.rect.height;
    await store.editText({
      page: pageNo,
      x: run.rect.x,
      y,
      width: run.rect.width,
      height: run.rect.height,
      font_size: Math.max(4, run.rect.height),
      new_text: next,
    });
  }

  return (
    <div
      ref={rootRef}
      className={`page-view${err ? " has-error" : ""}${app.editMode ? " edit-enabled" : ""}`}
      style={{ width: rendered?.sizePx.width, height: rendered?.sizePx.height }}>
      <canvas ref={canvasRef} width={rendered?.image.width} height={rendered?.image.height} />
      {err && <div className="page-error">{err}</div>}
      {!err && visible && (
        <TextLayer
          session={session}
          pageNo={pageNo}
          zoom={app.zoom}
          rotation={rotation}
          editing={!!editing}
          onStartEdit={(run, caretX) => {
            if (canEdit) setEditing({ run, caretX });
          }}
        />
      )}
      {editing && (
        <InlineEditor
          run={editing.run}
          zoom={app.zoom}
          caretX={editing.caretX}
          onCommit={(t) => void commitEdit(editing.run, t)}
          onCancel={() => setEditing(null)}
        />
      )}
    </div>
  );
}

export function Viewer() {
  const app = useAppState();
  const [err, setErr] = useState<string | null>(null);

  // Mirror the document selection into the store so Copy can enable/disable and
  // report a count (A-c10).
  useEffect(() => {
    const onSelectionChange = () => {
      store.setSelection(document.getSelection()?.toString() ?? "");
    };
    document.addEventListener("selectionchange", onSelectionChange);
    return () => document.removeEventListener("selectionchange", onSelectionChange);
  }, []);

  // Bind pdf.js once per document and publish the session to the store so the
  // sidebar thumbnails reuse it. Closing/replacing the doc destroys the binding.
  useEffect(() => {
    if (!app.doc) {
      setErr(null);
      store.setSession(null);
      return;
    }
    let alive = true;
    bind(app.doc)
      .then((s) => {
        if (alive) store.setSession(s);
        else void unbind(s); // StrictMode/dev double-invoke: drop the orphan
      })
      .catch((e) => alive && setErr(String(e)));
    return () => {
      alive = false;
      store.setSession(null);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [app.doc?.bytesB64]);

  const session = app.session;

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
    <div className="viewer-hint">Double-click a text line to edit it in place · Esc to cancel</div>
  );

  const pageCount = app.doc.pages.length;
  let body: ReactNode;
  if (app.viewMode === "single") {
    body = <PageView session={session} pageNo={app.currentPage} />;
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
              <PageView key={p} session={session} pageNo={p} />
            ))}
          </div>
        ))}
      </>
    );
  } else {
    body = (
      <>
        {app.doc.pages.map((p) => (
          <PageView key={p.page_no} session={session} pageNo={p.page_no} />
        ))}
      </>
    );
  }

  return (
    <div className="viewer-scroll">
      {overlay}
      {body}
    </div>
  );
}
