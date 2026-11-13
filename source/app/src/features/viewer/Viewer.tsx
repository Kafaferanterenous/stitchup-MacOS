import { useEffect, useLayoutEffect, useRef, useState, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";
import { useAppState, store } from "../../store";
import { bind, unbind, renderPage, getTextRuns, RenderSession, RenderedPage } from "../../adapters/renderAdapter";
import type { TextRun } from "../../types";
import { extractTo } from "../../actions";
import { PageOverlay } from "./PageOverlay";

// ------------------------------------------------------------ text metrics

let measureCtx: CanvasRenderingContext2D | null = null;

/** Width in CSS px of `text` at `sizePx` in `family`. */
function measure(text: string, sizePx: number, family: string): number {
  measureCtx ??= document.createElement("canvas").getContext("2d");
  if (!measureCtx) return text.length * sizePx * 0.5;
  measureCtx.font = `${sizePx}px ${family}`;
  return measureCtx.measureText(text).width;
}

/** Caret index in `text` nearest to `x` px from the run's left edge. */
function caretIndex(text: string, x: number, sizePx: number, family: string, scaleX: number): number {
  let best = 0;
  let bestD = Infinity;
  for (let i = 0; i <= text.length; i++) {
    const d = Math.abs(measure(text.slice(0, i), sizePx, family) * scaleX - x);
    if (d < bestD) {
      bestD = d;
      best = i;
    }
  }
  return best;
}

/**
 * Index of the editable run after (move=1) or before (move=-1) `from` in
 * reading order — by baseline (PDF y, top first) then x — so it works even when
 * `from` belongs to a previous render of the page.
 */
function neighbourRun(runs: TextRun[], from: TextRun, move: 1 | -1): number | null {
  const tol = Math.max(1, from.fontSize * 0.3);
  // >0 when `r` comes after `from` in reading order.
  const order = (r: TextRun) =>
    Math.abs(r.origin.y - from.origin.y) <= tol ? r.origin.x - from.origin.x : from.origin.y - r.origin.y;
  let best: number | null = null;
  let bestKey = Infinity;
  runs.forEach((r, i) => {
    if (!r.upright || !r.str.trim()) return;
    const k = order(r);
    // Skip the line itself (same baseline, same start).
    if (Math.abs(r.origin.y - from.origin.y) <= tol && Math.abs(r.origin.x - from.origin.x) < 0.5) return;
    if (move > 0 ? k > 0 && k < bestKey : k < 0 && -k < bestKey) {
      bestKey = Math.abs(k);
      best = i;
    }
  });
  return best;
}

/** Horizontal squeeze so the browser's glyphs span exactly the PDF run. */
function fitScale(run: TextRun, zoom: number): number {
  const natural = measure(run.str, run.fontSize * zoom, run.fontFamily);
  const target = run.pdfWidth * zoom;
  if (natural <= 0 || target <= 0) return 1;
  return Math.min(4, Math.max(0.25, target / natural));
}

// -------------------------------------------------------------- text layer

/**
 * Transparent text over the rendered page: native selection + copy (A-c10)
 * and in-place editing — double-click any line (as on Windows), right-click
 * for the context menu, or single-click while Edit Text mode is on. Spans are
 * scaled horizontally so the selection matches the rendered glyphs. Hidden on
 * rotated pages (the run geometry is extracted upright).
 */
function TextLayer({
  runs,
  zoom,
  editMode,
  canEdit,
  editingIndex,
  onStartEdit,
  onContext,
}: {
  runs: TextRun[];
  zoom: number;
  editMode: boolean;
  canEdit: boolean;
  editingIndex: number | null;
  onStartEdit: (index: number, caretX: number) => void;
  onContext: (index: number, x: number, y: number) => void;
}) {
  const startAt = (i: number, e: ReactMouseEvent<HTMLSpanElement>) => {
    e.preventDefault();
    e.stopPropagation();
    // A double-click also selected a word: drop it, we're editing now.
    window.getSelection()?.removeAllRanges();
    const box = e.currentTarget.getBoundingClientRect();
    onStartEdit(i, e.clientX - box.left);
  };
  return (
    <div className={`text-layer${editMode ? " editing" : ""}`}>
      {runs.map((run, i) => {
        const canEditRun = canEdit && run.upright && run.str.trim().length > 0;
        const editable = editMode && canEditRun;
        const sx = fitScale(run, zoom);
        return (
          <span
            key={i}
            data-run={i}
            className={`text-run${editable ? " editable" : ""}`}
            style={{
              left: run.rect.x * zoom,
              top: run.rect.y * zoom,
              height: run.rect.height * zoom,
              fontSize: run.fontSize * zoom,
              lineHeight: `${run.rect.height * zoom}px`,
              fontFamily: run.fontFamily,
              transform: `scaleX(${sx})`,
              visibility: editingIndex === i ? "hidden" : undefined,
            }}
            title={editable ? "Click to edit" : canEditRun ? "Double-click to edit · right-click for more" : undefined}
            onClick={editable ? (e) => startAt(i, e) : undefined}
            onDoubleClick={canEditRun ? (e) => startAt(i, e) : undefined}
            onContextMenu={
              canEditRun
                ? (e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    onContext(i, e.clientX, e.clientY);
                  }
                : undefined
            }>
            {run.str}
          </span>
        );
      })}
    </div>
  );
}

// ------------------------------------------------------------ inline editor

type Commit = { text: string; move: -1 | 0 | 1 };

/**
 * In-place editor over a text run. It covers the original glyphs, uses the
 * run's size and a matching family, grows while typing, and keeps showing the
 * new text until the page re-renders (no flash back to the old text).
 * Enter / click-away = save, Tab / Shift+Tab = save and edit next / previous
 * line, Esc = cancel. Clearing the text deletes the line.
 */
function InlineEditor({
  run,
  zoom,
  caret,
  pending,
  onCommit,
  onCancel,
}: {
  run: TextRun;
  zoom: number;
  caret: number;
  pending: string | null;
  onCommit: (c: Commit) => void;
  onCancel: () => void;
}) {
  const [text, setText] = useState(run.str);
  const ref = useRef<HTMLInputElement | null>(null);
  const done = useRef(false);
  const sizePx = run.fontSize * zoom;
  const sx = fitScale(run, zoom);
  const shown = pending ?? text;
  const width = Math.max(run.pdfWidth * zoom, measure(shown, sizePx, run.fontFamily) * sx) + sizePx * 0.6;

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || pending !== null) return;
    el.focus();
    el.setSelectionRange(caret, caret);
  }, [caret, pending]);

  function finish(move: Commit["move"]) {
    if (done.current) return;
    done.current = true;
    onCommit({ text, move });
  }

  return (
    <input
      ref={ref}
      className={`inline-editor${pending !== null ? " pending" : ""}`}
      value={shown}
      readOnly={pending !== null}
      spellCheck={false}
      aria-label="Edit text"
      style={{
        left: run.rect.x * zoom - 2,
        top: run.rect.y * zoom,
        width,
        height: run.rect.height * zoom,
        fontSize: sizePx,
        fontFamily: run.fontFamily,
        letterSpacing: sx !== 1 ? `${(sx - 1) * sizePx * 0.5}px` : undefined,
      }}
      onChange={(e) => setText(e.target.value)}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") {
          e.preventDefault();
          finish(0);
        } else if (e.key === "Tab") {
          e.preventDefault();
          finish(e.shiftKey ? -1 : 1);
        } else if (e.key === "Escape") {
          e.preventDefault();
          done.current = true;
          onCancel();
        }
      }}
      onBlur={() => finish(0)}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
    />
  );
}

// ------------------------------------------------------------- context menu

interface MenuItem {
  label: string;
  run?: () => void;
  disabled?: boolean;
}

/** Right-click menu (fixed position; closes on click-away, Esc, scroll). */
function ContextMenu({ x, y, items, onClose }: { x: number; y: number; items: (MenuItem | "-")[]; onClose: () => void }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState({ x, y });
  useLayoutEffect(() => {
    // Keep the menu on screen.
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({ x: Math.min(x, window.innerWidth - r.width - 4), y: Math.min(y, window.innerHeight - r.height - 4) });
    el.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [x, y]);
  useEffect(() => {
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      if (e.type === "mousedown" && ref.current?.contains(e.target as Node)) return;
      onClose();
    };
    window.addEventListener("mousedown", close, true);
    window.addEventListener("keydown", close, true);
    window.addEventListener("wheel", close, true);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("mousedown", close, true);
      window.removeEventListener("keydown", close, true);
      window.removeEventListener("wheel", close, true);
      window.removeEventListener("blur", close);
    };
  }, [onClose]);
  return (
    <div ref={ref} className="context-menu" role="menu" style={{ left: pos.x, top: pos.y }} onContextMenu={(e) => e.preventDefault()}>
      {items.map((it, i) =>
        it === "-" ? (
          <div key={i} className="context-sep" role="separator" />
        ) : (
          <button
            key={i}
            role="menuitem"
            disabled={it.disabled}
            onClick={() => {
              onClose();
              it.run?.();
            }}>
            {it.label}
          </button>
        ),
      )}
    </div>
  );
}

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const ta = document.createElement("textarea");
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }
  store.notify(`Copied ${text.length} character${text.length === 1 ? "" : "s"}`);
}

// ------------------------------------------------------------------ a page

function PageView({ session, pageNo }: { session: RenderSession; pageNo: number }) {
  const app = useAppState();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const [rendered, setRendered] = useState<RenderedPage | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);
  const [runs, setRuns] = useState<TextRun[]>([]);
  const [editing, setEditing] = useState<{ index: number; caret: number; pending: string | null } | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; run: number | null } | null>(null);
  // Index to open after the next re-render (Tab navigation).
  const nextEdit = useRef<{ from: TextRun; move: 1 | -1 } | null>(null);

  const meta = app.doc?.pages.find((p) => p.page_no === pageNo);
  const rotation = meta?.rotation ?? 0;
  const upright = rotation % 360 === 0;
  const canEdit = !!app.doc && !app.doc.passwordRequired;
  const inRange = pageNo <= session.pdf.numPages;

  // Rasterise only near the viewport (large documents).
  useEffect(() => {
    if (visible) return;
    const el = rootRef.current;
    if (!el || typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        setVisible(true);
        io.disconnect();
      }
    }, { rootMargin: "400px" });
    io.observe(el);
    return () => io.disconnect();
  }, [visible]);

  useEffect(() => {
    if (!visible || !inRange) return;
    let alive = true;
    renderPage(session, pageNo, app.zoom, rotation)
      .then((r) => {
        if (!alive) return;
        setErr(null);
        setRendered(r);
      })
      .catch((e) => alive && setErr(String(e)));
    return () => {
      alive = false;
    };
  }, [visible, inRange, session, pageNo, app.zoom, rotation]);

  // Text runs for selection / editing, refreshed with every new session.
  useEffect(() => {
    if (!visible || !inRange || !upright) {
      setRuns([]);
      return;
    }
    let alive = true;
    getTextRuns(session, pageNo)
      .then((r) => {
        if (!alive) return;
        setRuns(r);
        // A pending edit is done once the re-rendered text arrives.
        setEditing((ed) => (ed?.pending != null ? null : ed));
        const nx = nextEdit.current;
        if (nx) {
          nextEdit.current = null;
          const i = neighbourRun(r, nx.from, nx.move);
          if (i !== null) setEditing({ index: i, caret: r[i].str.length, pending: null });
        }
      })
      .catch(() => alive && setRuns([]));
    return () => {
      alive = false;
    };
  }, [visible, inRange, upright, session, pageNo]);

  // Paint after React applied the canvas backing size (resizing clears it).
  useEffect(() => {
    const c = canvasRef.current;
    if (!rendered || !c) return;
    c.getContext("2d")?.drawImage(rendered.image, 0, 0);
  }, [rendered]);

  // Leaving edit mode closes an open (non-pending) editor.
  useEffect(() => {
    if (!app.editMode) setEditing((ed) => (ed?.pending != null ? ed : null));
  }, [app.editMode]);

  async function commit(index: number, c: Commit) {
    const run = runs[index];
    if (!run) return setEditing(null);
    const next = c.text.replace(/\s+$/u, "");
    if (next === run.str.replace(/\s+$/u, "")) {
      setEditing(null);
      if (c.move) openNeighbour(run, c.move);
      return;
    }
    setEditing({ index, caret: 0, pending: next });
    if (c.move) nextEdit.current = { from: run, move: c.move };
    const res = await store.editText({
      page: pageNo,
      x: run.origin.x,
      y: run.origin.y,
      width: run.pdfWidth,
      font_size: run.fontSize,
      old_text: run.str,
      new_text: next,
    });
    if (!res) {
      nextEdit.current = null;
      setEditing(null);
    }
  }

  /** Unchanged text + Tab: move to the neighbouring line without saving. */
  function openNeighbour(from: TextRun, move: 1 | -1) {
    const i = neighbourRun(runs, from, move);
    if (i !== null) setEditing({ index: i, caret: runs[i].str.length, pending: null });
  }

  const size = rendered?.sizePx ?? {
    width: (rotation % 180 === 0 ? meta?.width_pt ?? 612 : meta?.height_pt ?? 792) * app.zoom,
    height: (rotation % 180 === 0 ? meta?.height_pt ?? 792 : meta?.width_pt ?? 612) * app.zoom,
  };
  const editRun = editing ? runs[editing.index] : null;

  function startEdit(index: number, x: number) {
    const run = runs[index];
    if (!run) return;
    const sizePx = run.fontSize * app.zoom;
    store.setCurrentFromScroll(pageNo);
    setEditing({ index, caret: caretIndex(run.str, x, sizePx, run.fontFamily, fitScale(run, app.zoom)), pending: null });
  }

  function menuItems(run: number | null): (MenuItem | "-")[] {
    const r = run !== null ? runs[run] : null;
    const sel = window.getSelection()?.toString() ?? "";
    const pages = app.doc?.pages.length ?? 0;
    const items: (MenuItem | "-")[] = [];
    if (r) {
      items.push(
        { label: "Edit Text", run: () => setEditing({ index: run!, caret: r.str.length, pending: null }), disabled: !canEdit || app.busy },
        { label: "Copy", run: () => void copyText(sel || r.str) },
        { label: "Copy Line", run: () => void copyText(r.str) },
        { label: `Find “${r.str.trim().slice(0, 24)}${r.str.trim().length > 24 ? "…" : ""}”`, run: () => {
            store.setQuery(r.str.trim());
            void store.find(r.str.trim());
          } },
        "-",
      );
    } else if (sel) {
      items.push({ label: "Copy", run: () => void copyText(sel) }, "-");
    }
    items.push(
      { label: app.editMode ? "Stop Editing Text" : "Edit Text Mode", run: () => store.setEditMode(!app.editMode), disabled: !canEdit },
      "-",
      { label: "Rotate Page Right", run: () => void store.rotate([pageNo], true), disabled: !canEdit || app.busy },
      { label: "Rotate Page Left", run: () => void store.rotate([pageNo], false), disabled: !canEdit || app.busy },
      { label: "Add Blank Page After", run: () => void store.insertBlank(pageNo), disabled: !canEdit || app.busy },
      { label: "Extract Page…", run: () => void extractTo([pageNo]), disabled: !canEdit },
      { label: "Delete Page", run: () => void store.deletePages([pageNo]), disabled: !canEdit || app.busy || pages <= 1 },
    );
    return items;
  }

  return (
    <div
      ref={rootRef}
      data-page={pageNo}
      className={`page-view${err ? " has-error" : ""}${app.editMode && canEdit ? " edit-enabled" : ""}`}
      style={{ width: size.width, height: size.height }}
      onContextMenu={(e) => {
        e.preventDefault();
        setMenu({ x: e.clientX, y: e.clientY, run: null });
      }}>
      <canvas ref={canvasRef} width={rendered?.image.width} height={rendered?.image.height} />
      {err && <div className="page-error">{err}</div>}
      {!err && (
        <div className="search-highlights">
          {app.searchHits.map((h, i) =>
            h.page_no === pageNo ? (
              <span
                key={i}
                className={`search-hit${i === app.searchIndex ? " current" : ""}`}
                style={{
                  left: h.rect.x * app.zoom,
                  top: h.rect.y * app.zoom,
                  width: Math.max(2, h.rect.width * app.zoom),
                  height: Math.max(2, h.rect.height * app.zoom),
                }}
              />
            ) : null,
          )}
        </div>
      )}
      {!err && upright && (
        <TextLayer
          runs={runs}
          zoom={app.zoom}
          editMode={app.editMode && canEdit && !app.busy}
          canEdit={canEdit && !app.busy}
          editingIndex={editing?.index ?? null}
          onStartEdit={startEdit}
          onContext={(index, x, y) => setMenu({ x, y, run: index })}
        />
      )}
      {!err && visible && <PageOverlay session={session} pageNo={pageNo} zoom={app.zoom} rotation={rotation} runs={runs} />}
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menuItems(menu.run)} onClose={() => setMenu(null)} />}
      {editing && editRun && (
        <InlineEditor
          key={`${editing.index}:${editRun.str}`}
          run={editRun}
          zoom={app.zoom}
          caret={editing.caret}
          pending={editing.pending}
          onCommit={(c) => void commit(editing.index, c)}
          onCancel={() => setEditing(null)}
        />
      )}
    </div>
  );
}

// ------------------------------------------------------------------ viewer

const TOOL_HINT: Record<string, string> = {
  highlight: "Highlight: drag across text",
  underline: "Underline: drag across text",
  strikeout: "Strike-out: drag across text",
  note: "Sticky note: click where it goes",
  textbox: "Text box: drag the box (or click), then type",
  rect: "Rectangle: drag on the page",
  oval: "Oval: drag on the page",
  line: "Line: drag from start to end",
  arrow: "Arrow: drag from start to the point",
  pen: "Pen: draw freehand",
  stamp: "Stamp: click or drag where it goes",
  link: "Link: drag the clickable area",
  redact: "Redact: drag over content to remove it permanently",
  select: "Select: click or drag a box · ⌘-click adds · drag to move · ⌫ deletes",
  "shape-rect": "Insert rectangle into the page: drag",
  "shape-oval": "Insert oval into the page: drag",
  "shape-line": "Insert line into the page: drag",
  "shape-arrow": "Insert arrow into the page: drag",
  sign: "Signature: drag the box where the signature appears",
  image: "Image: click to place at natural size, or drag to set its width",
};

export function Viewer() {
  const app = useAppState();
  const [err, setErr] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const programmaticUntil = useRef(0);

  // Measure the scrolling viewport for fit-page / fit-width.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const m = () => store.setViewport(el.clientWidth, el.clientHeight);
    m();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", m);
      return () => window.removeEventListener("resize", m);
    }
    const ro = new ResizeObserver(m);
    ro.observe(el);
    return () => ro.disconnect();
  }, [app.session !== null]);

  // Mirror the document selection for Copy (A-c10).
  useEffect(() => {
    const onSel = () => store.setSelection(document.getSelection()?.toString() ?? "");
    document.addEventListener("selectionchange", onSel);
    return () => document.removeEventListener("selectionchange", onSel);
  }, []);

  // Fit: single view fits the current page; continuous/spread fit the largest
  // page (so the zoom doesn't jump while scrolling a mixed-size document).
  useEffect(() => {
    const pages = app.doc?.pages;
    if (!pages?.length) return;
    const vp = app.viewport;
    if (vp.width <= 0 || vp.height <= 0) return;
    const dims = (p: (typeof pages)[number]) => {
      const swap = ((p.rotation % 180) + 180) % 180 === 90;
      return { width: swap ? p.height_pt : p.width_pt, height: swap ? p.width_pt : p.height_pt };
    };
    let box: { width: number; height: number };
    if (app.viewMode === "single") {
      box = dims(pages.find((p) => p.page_no === app.currentPage) ?? pages[0]);
    } else {
      box = pages.map(dims).reduce((a, b) => ({ width: Math.max(a.width, b.width), height: Math.max(a.height, b.height) }));
      if (app.viewMode === "spread") box = { width: box.width * 2 + 16, height: box.height };
    }
    store.applyFit(vp, box);
  }, [app.viewport, app.zoomMode, app.viewMode, app.viewMode === "single" ? app.currentPage : 0, app.doc?.pages]);

  // Bind pdf.js per document version. The previous session stays on screen
  // until the new one is ready, so edits don't flash or lose the scroll spot.
  useEffect(() => {
    if (!app.doc) {
      setErr(null);
      store.setSession(null);
      return;
    }
    let alive = true;
    bind(app.doc)
      .then((s) => {
        if (alive) {
          setErr(null);
          store.setSession(s);
        } else void unbind(s);
      })
      .catch((e) => alive && setErr(String(e)));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [app.doc?.bytesB64, app.doc?.path]);

  // Scroll the current page into view when asked (thumbnail, Go to, search,
  // a newly opened document). A request made while the previous document's
  // session is still on screen waits until the new one is bound.
  const scrolledFor = useRef(-1);
  useEffect(() => {
    if (scrolledFor.current === app.scrollSeq) return;
    if (!app.session || app.session.doc.path !== app.doc?.path) return;
    if (app.viewMode === "single") {
      scrolledFor.current = app.scrollSeq;
      return;
    }
    const el = scrollRef.current?.querySelector<HTMLElement>(`[data-page="${app.currentPage}"]`);
    if (!el) return;
    scrolledFor.current = app.scrollSeq;
    programmaticUntil.current = Date.now() + 400;
    el.scrollIntoView({ block: "start" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [app.scrollSeq, app.session]);

  // Keep the current search hit visible.
  useEffect(() => {
    if (app.searchIndex < 0) return;
    const t = setTimeout(() => {
      const hit = scrollRef.current?.querySelector<HTMLElement>(".search-hit.current");
      if (hit) {
        programmaticUntil.current = Date.now() + 400;
        hit.scrollIntoView({ block: "center", inline: "nearest" });
      }
    }, 60);
    return () => clearTimeout(t);
  }, [app.searchIndex, app.searchHits]);

  // Track which page is under the upper third of the viewport while scrolling.
  function onScroll() {
    const host = scrollRef.current;
    if (!host || app.viewMode === "single" || Date.now() < programmaticUntil.current) return;
    const line = host.getBoundingClientRect().top + host.clientHeight / 3;
    for (const el of host.querySelectorAll<HTMLElement>("[data-page]")) {
      const r = el.getBoundingClientRect();
      if (r.top <= line && r.bottom >= line) {
        store.setCurrentFromScroll(Number(el.dataset.page));
        return;
      }
    }
  }

  const session = app.session;
  if (!app.doc)
    return (
      <div className="empty-viewer">
        <b>No document open</b>
        <span>Use File – Open (⌘O) to open a PDF, or drop one here</span>
      </div>
    );
  if (app.doc.passwordRequired) return <div className="empty-viewer">🔒 Password required to view this document</div>;
  if (err) return <div className="viewer-error">Renderer failed: {err}</div>;
  if (!session) return <div className="empty-viewer">Parsing document…</div>;

  const pageCount = app.doc.pages.length;
  let body: ReactNode;
  if (app.viewMode === "single") {
    body = <PageView session={session} pageNo={Math.min(app.currentPage, pageCount)} />;
  } else if (app.viewMode === "spread") {
    const pairs: number[][] = [];
    for (let i = 1; i <= pageCount; i += 2) pairs.push(i < pageCount ? [i, i + 1] : [i]);
    body = pairs.map((pair) => (
      <div className="page-spread" key={pair.join("-")}>
        {pair.map((p) => (
          <PageView key={p} session={session} pageNo={p} />
        ))}
      </div>
    ));
  } else {
    body = app.doc.pages.map((p) => <PageView key={p.page_no} session={session} pageNo={p.page_no} />);
  }

  return (
    <div className="viewer-scroll" ref={scrollRef} onScroll={onScroll}>
      {app.tool && <div className="viewer-hint tool-hint">{TOOL_HINT[app.tool]} · Esc to stop</div>}
      {app.editMode && (
        <div className="viewer-hint">
          Edit Text: click any line to edit it in place · Enter or click away to save · Tab = next line · Esc = cancel · clear the text to delete it
        </div>
      )}
      {body}
    </div>
  );
}
