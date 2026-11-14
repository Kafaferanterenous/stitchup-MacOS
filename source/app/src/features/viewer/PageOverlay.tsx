//! Interactive layer over a rendered page:
//!  - no tool: clickable links (web / page) and sticky-note popups;
//!  - an Annotate tool: drag (or click) to place it — Highlight / Underline /
//!    Strike-out snap to the text under the drag, Pen draws freehand;
//!  - Select (Content): pick text runs, images and annotations (click,
//!    ⌘-click, marquee), drag to move, ⌫ to delete.
//! Geometry goes through the pdf.js viewport, so it is right at any zoom
//! and page rotation.

import { useEffect, useRef, useState, type PointerEvent as RPointerEvent } from "react";
import type { PageViewport } from "pdfjs-dist";
import { store, useAppState, rgb, annotColor, type Tool } from "../../store";
import type { RenderSession } from "../../adapters/renderAdapter";
import type { TextRun } from "../../types";
import { cmdListFields, cmdListObjects, cmdOpenUrl, type FormField, type PageObject } from "../../ipc";

interface PdfAnnot {
  obj: number | null;
  subtype: string;
  rect: number[];
  url?: string;
  dest?: unknown;
  contents?: string;
}

type Box = [number, number, number, number]; // css px x0,y0,x1,y1

const DRAW_TOOLS: Tool[] = [
  "highlight", "underline", "strikeout", "note", "textbox", "rect", "oval", "line", "arrow", "pen", "stamp", "link", "redact",
  "shape-rect", "shape-oval", "shape-line", "shape-arrow", "sign", "image",
];

const norm = (b: Box): Box => [Math.min(b[0], b[2]), Math.min(b[1], b[3]), Math.max(b[0], b[2]), Math.max(b[1], b[3])];
const hits = (a: Box, b: Box) => a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];

export function PageOverlay({
  session,
  pageNo,
  zoom,
  rotation,
  runs,
}: {
  session: RenderSession;
  pageNo: number;
  zoom: number;
  rotation: number;
  runs: TextRun[];
}) {
  const app = useAppState();
  const tool = app.tool;
  const [vp, setVp] = useState<PageViewport | null>(null);
  const [annots, setAnnots] = useState<PdfAnnot[]>([]);
  const [objects, setObjects] = useState<PageObject[]>([]);
  const [drag, setDrag] = useState<{ start: [number, number]; cur: [number, number]; moving?: boolean } | null>(null);
  const [ink, setInk] = useState<[number, number][]>([]);
  const [note, setNote] = useState<PdfAnnot | null>(null);
  const [fields, setFields] = useState<FormField[]>([]);
  const hostRef = useRef<HTMLDivElement | null>(null);

  // Viewport (zoom + rotation) for coordinate conversion.
  useEffect(() => {
    let alive = true;
    session.pdf.getPage(pageNo).then((p) => alive && setVp(p.getViewport({ scale: zoom, rotation })), () => {});
    return () => {
      alive = false;
    };
  }, [session, pageNo, zoom, rotation]);

  // Annotations on the page (links, notes, everything selectable).
  useEffect(() => {
    setAnnots([]);
    let alive = true;
    session.pdf
      .getPage(pageNo)
      .then((p) => p.getAnnotations())
      .then((list) => {
        if (!alive) return;
        setAnnots(
          (list as Array<Record<string, unknown>>)
            .filter((a) => a.annotationType !== 20 /* Widget */ || false)
            .map((a) => ({
              obj: typeof a.id === "string" && /^\d+R$/.test(a.id) ? Number((a.id as string).slice(0, -1)) : null,
              subtype: String(a.subtype ?? ""),
              rect: a.rect as number[],
              url: (a.url as string) ?? (a.unsafeUrl as string) ?? undefined,
              dest: a.dest,
              contents: (a.contentsObj as { str?: string } | undefined)?.str ?? (a.contents as string) ?? "",
            })),
        );
      })
      .catch(() => alive && setAnnots([]));
    return () => {
      alive = false;
    };
  }, [session, pageNo]);

  // Form fields (document-wide list; this page's widgets are drawn below).
  useEffect(() => {
    let alive = true;
    cmdListFields().then((f) => alive && setFields(f), () => alive && setFields([]));
    return () => {
      alive = false;
    };
  }, [session]);

  // Content objects, only while the Select tool is armed.
  useEffect(() => {
    if (tool !== "select") {
      setObjects([]);
      return;
    }
    // Ids are positional: drop the old list at once so a stale outline can
    // never be clicked while the new one loads.
    setObjects([]);
    let alive = true;
    cmdListObjects(pageNo).then((o) => alive && setObjects(o), () => alive && setObjects([]));
    return () => {
      alive = false;
    };
  }, [tool, session, pageNo]);

  if (!vp) return null;
  // Colour of the shape being dragged: content shapes use Fore, annotations Annotate ▸ Colour.
  const liveColor = (tool ?? "").startsWith("shape-") ? app.foreColor : annotColor(app, tool === "highlight" ? "highlight" : "other");

  // Between an edit and the re-render, the page on screen is the previous
  // version: object ids/outlines are stale, so nothing is pickable.
  const stale = app.busy || session.doc.bytesB64 !== app.doc?.bytesB64;
  const live = stale ? [] : objects;

  const toPdf = (x: number, y: number) => vp.convertToPdfPoint(x, y) as [number, number];
  const pdfRect = (b: Box): [number, number, number, number] => {
    const [ax, ay] = toPdf(b[0], b[1]);
    const [bx, by] = toPdf(b[2], b[3]);
    return [Math.min(ax, bx), Math.min(ay, by), Math.max(ax, bx), Math.max(ay, by)];
  };
  const viewBox = (r: number[]): Box => norm(vp.convertToViewportRectangle(r) as Box);
  const local = (e: RPointerEvent): [number, number] => {
    const r = hostRef.current!.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top];
  };

  const picked = app.picked?.page === pageNo ? app.picked : null;

  // ---------------------------------------------------------- finishing
  function finish(box: Box, click: boolean) {
    const t = tool!;
    const page = pageNo;
    const pr = pdfRect(box);
    const fore = rgb(app.foreColor);
    const back = app.backColor ? rgb(app.backColor) : null;
    const ann = rgb(annotColor(app, "other"));
    const w = app.strokeWidth;
    const pt = toPdf(box[0], box[1]);
    const def = (wPt: number, hPt: number): [number, number, number, number] => [pt[0], pt[1] - hPt, pt[0] + wPt, pt[1]];
    switch (t) {
      case "highlight":
      case "underline":
      case "strikeout": {
        const boxes = markupBoxes(box).map(pdfRect);
        if (!boxes.length) return store.notify("Drag across some text");
        const kind = t === "strikeout" ? "strike_out" : t;
        void store.annotate(page, { kind, boxes, color: rgb(annotColor(app, t === "highlight" ? "highlight" : "other")) });
        return;
      }
      case "note":
        store.setPendingInput({ kind: "note", page, rect: [pt[0], pt[1], pt[0], pt[1]] });
        return;
      case "textbox":
        store.setPendingInput({ kind: "textbox", page, rect: click ? def(200, 40) : pr });
        return;
      case "link":
        store.setPendingInput({ kind: "link", page, rect: click ? def(120, 18) : pr });
        return;
      case "sign":
        store.setPendingInput({ kind: "sign", page, rect: click ? def(220, 64) : pr });
        return;
      case "image": {
        const img = app.pendingImage;
        if (!img) return;
        // Keep the picture's proportions: the drag sets the width.
        const aspect = img.height / img.width;
        const wPt = click ? Math.min(img.width * 0.75, 300) : Math.max(8, pr[2] - pr[0]);
        const hPt = wPt * aspect;
        void store.placeImage(page, [pt[0], pt[1] - hPt, pt[0] + wPt, pt[1]]);
        return;
      }
      case "stamp":
        void store.annotate(page, { kind: "stamp", rect: click ? def(160, 48) : pr, label: app.stampLabel, color: stampColor(app.stampLabel) });
        return;
      case "rect":
      case "oval":
        void store.annotate(page, { kind: t === "rect" ? "square" : "circle", rect: click ? def(120, 80) : pr, color: ann, fill: back, width: w });
        return;
      case "line":
      case "arrow": {
        const a = toPdf(box[0], box[1]);
        const b = click ? [a[0] + 120, a[1]] : toPdf(box[2], box[3]);
        void store.annotate(page, { kind: "line", from: a, to: b, color: ann, width: w, arrow: t === "arrow" });
        return;
      }
      case "redact":
        if (click) return store.notify("Drag over the area to redact");
        void store.redact(page, pr);
        return;
      case "shape-rect":
      case "shape-oval":
      case "shape-line":
      case "shape-arrow": {
        const kind = t.slice(6);
        let rect: number[];
        if (kind === "line" || kind === "arrow") {
          const a = toPdf(box[0], box[1]);
          const b = click ? [a[0] + 120, a[1]] : toPdf(box[2], box[3]);
          rect = [a[0], a[1], b[0], b[1]];
        } else rect = click ? def(120, 80) : pr;
        void store.insertShape(page, { kind, rect, stroke: fore, fill: back, width: w });
        return;
      }
    }
  }

  /** Text-run pieces under a drag box (css px), for markup tools. */
  function markupBoxes(box: Box): Box[] {
    const b = norm(box);
    const out: Box[] = [];
    if (rotation % 360 === 0) {
      for (const r of runs) {
        const rb: Box = [r.rect.x * zoom, r.rect.y * zoom, (r.rect.x + r.rect.width) * zoom, (r.rect.y + r.rect.height) * zoom];
        // A run counts when the drag overlaps it vertically (a flat drag along
        // a line has zero height) and horizontally.
        if (rb[3] < b[1] || rb[1] > b[3] || rb[2] < b[0] || rb[0] > b[2] || !r.str.trim()) continue;
        out.push([Math.max(rb[0], b[0]), rb[1], Math.min(rb[2], b[2]), rb[3]]);
      }
    }
    if (!out.length && b[2] - b[0] > 3 && b[3] - b[1] > 3) out.push(b);
    return out;
  }

  // ------------------------------------------------------------ events
  function down(e: RPointerEvent) {
    if (e.button !== 0 || stale) return;
    e.preventDefault();
    e.stopPropagation();
    (e.target as Element).setPointerCapture?.(e.pointerId);
    const p = local(e);
    if (tool === "pen") {
      setInk([p]);
      return;
    }
    if (tool === "select") {
      const hit = hitTest(p);
      if (hit && picked && isPicked(hit)) {
        setDrag({ start: p, cur: p, moving: true });
        return;
      }
      if (hit) {
        const base = e.metaKey || e.ctrlKey ? picked ?? { page: pageNo, objects: [], annots: [] } : { page: pageNo, objects: [], annots: [] };
        const next = toggle(base, hit);
        store.setPicked(next);
        setDrag({ start: p, cur: p, moving: !(e.metaKey || e.ctrlKey) });
        return;
      }
      if (!(e.metaKey || e.ctrlKey)) store.setPicked(null);
    }
    setDrag({ start: p, cur: p });
  }

  function move(e: RPointerEvent) {
    const p = local(e);
    if (tool === "pen" && ink.length) {
      setInk((s) => [...s, p]);
      return;
    }
    if (drag) setDrag({ ...drag, cur: p });
  }

  function up(e: RPointerEvent) {
    const p = local(e);
    if (tool === "pen" && ink.length) {
      const pts = [...ink, p];
      setInk([]);
      if (pts.length < 2) return;
      void store.annotate(pageNo, { kind: "ink", strokes: [pts.map(([x, y]) => toPdf(x, y))], color: rgb(annotColor(app, "other")), width: app.strokeWidth });
      return;
    }
    if (!drag) return;
    const d = drag;
    setDrag(null);
    const box: Box = [d.start[0], d.start[1], p[0], p[1]];
    const click = Math.abs(p[0] - d.start[0]) < 4 && Math.abs(p[1] - d.start[1]) < 4;
    if (tool === "select") {
      if (d.moving) {
        if (click) return;
        const a = toPdf(d.start[0], d.start[1]);
        const b = toPdf(p[0], p[1]);
        void store.movePicked(b[0] - a[0], b[1] - a[1]);
        return;
      }
      if (click) return;
      // Marquee: everything the box touches.
      const m = norm(box);
      const base = e.metaKey || e.ctrlKey ? picked ?? { page: pageNo, objects: [], annots: [] } : { page: pageNo, objects: [], annots: [] };
      const objs = live.filter((o) => hits(viewBox(o.rect), m)).map((o) => o.id);
      const ans = annots.filter((a) => a.obj !== null && hits(viewBox(a.rect), m)).map((a) => a.obj!);
      store.setPicked({ page: pageNo, objects: [...new Set([...base.objects, ...objs])], annots: [...new Set([...base.annots, ...ans])] });
      return;
    }
    finish(norm(box), click);
  }

  type Hit = { kind: "obj"; id: string } | { kind: "annot"; obj: number };
  function hitTest(p: [number, number]): Hit | null {
    const inBox = (b: Box) => p[0] >= b[0] - 2 && p[0] <= b[2] + 2 && p[1] >= b[1] - 2 && p[1] <= b[3] + 2;
    // Annotations sit on top of content.
    for (const a of [...annots].reverse()) if (a.obj !== null && inBox(viewBox(a.rect))) return { kind: "annot", obj: a.obj };
    // Smallest content object under the pointer (text over its background image).
    const under = live.filter((o) => inBox(viewBox(o.rect)));
    under.sort((a, b) => area(viewBox(a.rect)) - area(viewBox(b.rect)));
    return under.length ? { kind: "obj", id: under[0].id } : null;
  }
  const area = (b: Box) => (b[2] - b[0]) * (b[3] - b[1]);
  const isPicked = (h: Hit) => (h.kind === "obj" ? picked!.objects.includes(h.id) : picked!.annots.includes(h.obj));
  function toggle(base: NonNullable<typeof picked>, h: Hit) {
    const objects = [...base.objects];
    const annotsSel = [...base.annots];
    if (h.kind === "obj") {
      const i = objects.indexOf(h.id);
      i >= 0 ? objects.splice(i, 1) : objects.push(h.id);
    } else {
      const i = annotsSel.indexOf(h.obj);
      i >= 0 ? annotsSel.splice(i, 1) : annotsSel.push(h.obj);
    }
    return { page: pageNo, objects, annots: annotsSel };
  }

  async function followLink(a: PdfAnnot) {
    if (a.url) {
      await cmdOpenUrl(a.url).catch(() => window.open(a.url, "_blank"));
      store.notify(`Opening ${a.url}`);
      return;
    }
    try {
      const dest = typeof a.dest === "string" ? await session.pdf.getDestination(a.dest) : (a.dest as unknown[]);
      if (!dest) return;
      const ref = dest[0] as { num: number; gen: number } | number;
      const idx = typeof ref === "number" ? ref : await session.pdf.getPageIndex(ref as never);
      store.goToPage(idx + 1);
    } catch {
      /* broken link */
    }
  }

  const drawing = tool && DRAW_TOOLS.includes(tool);
  const dragBox = drag && !drag.moving ? norm([drag.start[0], drag.start[1], drag.cur[0], drag.cur[1]]) : null;
  const moveDelta = drag?.moving ? [drag.cur[0] - drag.start[0], drag.cur[1] - drag.start[1]] : [0, 0];

  return (
    <div
      ref={hostRef}
      className={`page-overlay${drawing ? " drawing" : ""}${tool === "select" ? " selecting" : ""}`}
      data-tool={tool ?? ""}
      data-ready={tool === "select" ? String(live.length > 0) : undefined}
      onPointerDown={tool ? down : undefined}
      onPointerMove={tool ? move : undefined}
      onPointerUp={tool ? up : undefined}>
      {/* Reading mode: links and notes */}
      {!tool &&
        annots
          .filter((a) => (a.subtype === "Link" && (a.url || a.dest)) || a.subtype === "Text")
          .map((a, i) => {
            const b = viewBox(a.rect);
            return (
              <button
                key={i}
                type="button"
                className={`annot-hot ${a.subtype === "Link" ? "link" : "note"}`}
                style={{ left: b[0], top: b[1], width: b[2] - b[0], height: b[3] - b[1] }}
                title={a.subtype === "Link" ? a.url ?? "Go to page" : a.contents}
                aria-label={a.subtype === "Link" ? `Link ${a.url ?? ""}` : "Note"}
                onClick={(e) => {
                  e.stopPropagation();
                  if (a.subtype === "Link") void followLink(a);
                  else setNote(note === a ? null : a);
                }}
              />
            );
          })}
      {note && (
        <div className="note-popup" style={{ left: viewBox(note.rect)[2] + 6, top: viewBox(note.rect)[1] }} role="dialog" aria-label="Note">
          <div>{note.contents || "(empty note)"}</div>
          <button type="button" onClick={() => setNote(null)}>
            Close
          </button>
        </div>
      )}

      {/* Fillable form fields (reading mode) */}
      {!tool && !app.editMode && !stale && <FormLayer fields={fields} pageNo={pageNo} viewBox={viewBox} zoom={zoom} />}

      {/* Select tool: outlines of everything pickable */}
      {tool === "select" &&
        live.map((o) => {
          const b = viewBox(o.rect);
          const on = picked?.objects.includes(o.id);
          return (
            <div
              key={o.id}
              data-obj={o.id}
              className={`obj-box ${o.kind}${on ? " picked" : ""}`}
              title={o.kind === "text" ? o.text : o.kind}
              style={{ left: b[0], top: b[1], width: b[2] - b[0], height: b[3] - b[1], transform: on ? `translate(${moveDelta[0]}px, ${moveDelta[1]}px)` : undefined }}
            />
          );
        })}
      {tool === "select" &&
        annots
          .filter((a) => a.obj !== null)
          .map((a) => {
            const b = viewBox(a.rect);
            const on = picked?.annots.includes(a.obj!);
            return (
              <div
                key={`a${a.obj}`}
                data-annot={a.obj!}
                className={`obj-box annot${on ? " picked" : ""}`}
                title={a.subtype}
                style={{ left: b[0], top: b[1], width: b[2] - b[0], height: b[3] - b[1], transform: on ? `translate(${moveDelta[0]}px, ${moveDelta[1]}px)` : undefined }}
              />
            );
          })}

      {/* Live previews */}
      {dragBox && (tool === "line" || tool === "arrow" || tool === "shape-line" || tool === "shape-arrow") ? (
        <svg className="draw-preview">
          <line x1={drag!.start[0]} y1={drag!.start[1]} x2={drag!.cur[0]} y2={drag!.cur[1]} stroke={liveColor} strokeWidth={app.strokeWidth * zoom} />
        </svg>
      ) : (
        dragBox && (
          <div
            className={`drag-box ${tool === "redact" ? "redact" : tool === "highlight" ? "highlight" : ""}${tool === "oval" || tool === "shape-oval" ? " oval" : ""}`}
            style={{ left: dragBox[0], top: dragBox[1], width: dragBox[2] - dragBox[0], height: dragBox[3] - dragBox[1], borderColor: tool === "select" ? undefined : liveColor }}
          />
        )
      )}
      {ink.length > 1 && (
        <svg className="draw-preview">
          <polyline points={ink.map((p) => p.join(",")).join(" ")} fill="none" stroke={liveColor} strokeWidth={app.strokeWidth * zoom} strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      )}
    </div>
  );
}

/** Stamp colours: green for positive, red for negative, blue otherwise. */
export function stampColor(label: string): number[] {
  const l = label.toLowerCase();
  if (/approved|paid|final|received|completed|accepted/.test(l)) return [0.1, 0.55, 0.2];
  if (/rejected|void|draft|confidential|not approved|urgent/.test(l)) return [0.8, 0.1, 0.1];
  return [0.1, 0.3, 0.75];
}


/** HTML controls over the page's form widgets; every change is saved into the
 *  PDF (value + regenerated appearance) and is undoable. */
function FormLayer({
  fields,
  pageNo,
  viewBox,
  zoom,
}: {
  fields: FormField[];
  pageNo: number;
  viewBox: (r: number[]) => Box;
  zoom: number;
}) {
  const items = fields.flatMap((f) => f.widgets.filter((w) => w.page === pageNo).map((w) => ({ f, w })));
  if (!items.length) return null;
  return (
    <>
      {items.map(({ f, w }, i) => {
        if (f.kind === "signature" || f.kind === "button") return null;
        const b = viewBox(w.rect);
        const style = { left: b[0], top: b[1], width: b[2] - b[0], height: b[3] - b[1] };
        const fontSize = Math.max(8, Math.min(14 * zoom, (b[3] - b[1]) * 0.7));
        const key = `${f.name}#${i}`;
        if (f.kind === "checkbox") {
          const on = f.value !== "" && f.value !== "Off";
          return (
            <button
              key={key}
              type="button"
              role="checkbox"
              aria-checked={on}
              aria-label={f.name}
              disabled={f.readonly}
              className="form-field toggle"
              style={style}
              onClick={() => void store.setField(f.name, !on)}
            />
          );
        }
        if (f.kind === "radio") {
          const on = !!w.on_state && f.value === w.on_state;
          return (
            <button
              key={key}
              type="button"
              role="radio"
              aria-checked={on}
              aria-label={`${f.name}: ${optionLabel(f, w.on_state)}`}
              title={optionLabel(f, w.on_state)}
              disabled={f.readonly}
              className="form-field toggle radio"
              style={style}
              onClick={() => w.on_state && !on && void store.setField(f.name, w.on_state)}
            />
          );
        }
        if (f.kind === "choice") {
          return (
            <select
              key={key}
              aria-label={f.name}
              className="form-field"
              style={{ ...style, fontSize }}
              disabled={f.readonly}
              value={f.value}
              onChange={(e) => void store.setField(f.name, e.target.value)}>
              {!f.options.includes(f.value) && <option value={f.value}>{f.value}</option>}
              {f.options.map((o) => (
                <option key={o} value={o}>
                  {o}
                </option>
              ))}
            </select>
          );
        }
        return <TextField key={key} f={f} style={{ ...style, fontSize }} />;
      })}
    </>
  );
}

function TextField({ f, style }: { f: FormField; style: React.CSSProperties }) {
  const [v, setV] = useState(f.value);
  useEffect(() => setV(f.value), [f.value]);
  const commit = () => {
    if (v !== f.value) void store.setField(f.name, v);
  };
  const common = {
    "aria-label": f.name,
    className: "form-field",
    style,
    value: v,
    readOnly: f.readonly,
    onChange: (e: { target: { value: string } }) => setV(e.target.value),
    onBlur: commit,
  };
  return f.multiline ? (
    <textarea {...common} />
  ) : (
    <input {...common} onKeyDown={(e) => e.key === "Enter" && (e.currentTarget as HTMLInputElement).blur()} />
  );
}

/** Readable radio option: pdf-lib & others name states "0","1"… and keep the
 *  real names in /Opt. */
function optionLabel(f: FormField, on: string | null): string {
  if (!on) return "";
  return /^\d+$/.test(on) && f.options[Number(on)] ? f.options[Number(on)] : on;
}
