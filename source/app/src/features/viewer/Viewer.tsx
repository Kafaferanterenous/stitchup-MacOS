import { useEffect, useRef, useState } from "react";
import { useAppState } from "../../store";
import {
  bind,
  renderPage,
  unbind,
  RenderSession,
  RenderedPage,
} from "../../adapters/renderAdapter";

function PageView({ session, pageNo }: { session: RenderSession; pageNo: number }) {
  const app = useAppState();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [rendered, setRendered] = useState<RenderedPage | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const docMeta = session.doc.pages.find((p) => p.page_no === pageNo);
  const rotation = docMeta?.rotation ?? 0;

  useEffect(() => {
    let alive = true;
    const scale = app.zoomMode === "actual" ? app.zoom : app.zoom;
    renderPage(session, pageNo, scale, rotation)
      .then((r) => {
        if (!alive) return;
        setRendered(r);
        if (canvasRef.current) {
          const ctx = canvasRef.current.getContext("2d");
          ctx?.drawImage(r.image, 0, 0);
        }
      })
      .catch((e) => alive && setErr(String(e)));
    return () => {
      alive = false;
    };
  }, [session, pageNo, app.zoom, app.zoomMode, rotation]);

  return (
    <div
      className={`page-view${err ? " has-error" : ""}`}
      style={{ width: rendered?.sizePx.width, height: rendered?.sizePx.height }}
    >
      <canvas ref={canvasRef} width={rendered?.sizePx.width} height={rendered?.sizePx.height} />
      {err && <div className="page-error">{err}</div>}
    </div>
  );
}

export function Viewer() {
  const app = useAppState();
  const [session, setSession] = useState<RenderSession | null>(null);
  const [err, setErr] = useState<string | null>(null);

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

  if (app.viewMode === "single") {
    return (
      <div className="viewer-scroll">
        <PageView key={app.currentPage} session={session} pageNo={app.currentPage} />
      </div>
    );
  }

  return (
    <div className="viewer-scroll">
      {app.doc.pages.map((p) => (
        <PageView key={p.page_no} session={session} pageNo={p.page_no} />
      ))}
    </div>
  );
}