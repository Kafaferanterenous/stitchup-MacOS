import {
  DndContext,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useEffect, useRef, useState, type MouseEvent } from "react";
import { extractTo } from "../../actions";
import { Icon } from "../ribbon/icons";
import { useAppState, store } from "../../store";
import { renderPage, type RenderSession } from "../../adapters/renderAdapter";

/**
 * Live page preview: renders the page through the SAME pdf.js session as the
 * viewer (shared via the store) so nothing is parsed twice. The canvas is only
 * rasterised once the thumbnail scrolls near the viewport (large-document F-11).
 */
function PagePreview({
  session,
  pageNo,
  width,
  rotation,
}: {
  session: RenderSession;
  pageNo: number;
  width: number;
  rotation: number;
}) {
  const hostRef = useRef<HTMLSpanElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [shown, setShown] = useState(false);

  useEffect(() => {
    if (shown) return;
    const el = hostRef.current;
    if (!el || typeof IntersectionObserver === "undefined") {
      setShown(true);
      return;
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setShown(true);
          io.disconnect();
        }
      },
      { rootMargin: "300px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [shown]);

  useEffect(() => {
    if (!shown) return;
    const base = session.doc.pages.find((p) => p.page_no === pageNo);
    const widthPt = base?.width_pt ?? 612;
    const scale = width / Math.max(1, widthPt);
    let alive = true;
    renderPage(session, pageNo, scale, rotation)
      .then((r) => {
        if (!alive) return;
        const c = canvasRef.current;
        if (!c) return;
        c.width = r.image.width;
        c.height = r.image.height;
        c.getContext("2d")?.drawImage(r.image, 0, 0);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [shown, session, pageNo, width, rotation]);

  return (
    <span ref={hostRef} className="thumb-sheet">
      <canvas ref={canvasRef} className="thumb-canvas" />
    </span>
  );
}

function Thumb({
  pageNo,
  picked,
  onPick,
}: {
  pageNo: number;
  picked: boolean;
  onPick: (pageNo: number, e: MouseEvent) => void;
}) {
  const app = useAppState();
  const { attributes, listeners, setNodeRef, transform, transition } = useSortable({
    id: pageNo,
  });
  const base = app.doc?.pages.find((p) => p.page_no === pageNo);
  const selected = app.currentPage === pageNo;
  const bookmarked = app.doc?.bookmarks.includes(pageNo) ?? false;
  const hostRef = useRef<HTMLButtonElement | null>(null);

  return (
    <button
      ref={(el) => {
        hostRef.current = el;
        setNodeRef(el);
      }}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      {...attributes}
      {...listeners}
      className={`thumb${selected ? " selected" : ""}${picked ? " picked" : ""}${bookmarked ? " bookmarked" : ""}`}
      aria-pressed={picked}
      onClick={(e) => onPick(pageNo, e)}
      title={`Page ${pageNo}${base?.rotation ? ` · rotated ${base.rotation}°` : ""} · ⌘-click / Shift-click to select several`}
    >
      {app.session ? (
        <PagePreview
          session={app.session}
          pageNo={pageNo}
          width={150}
          rotation={base?.rotation ?? 0}
        />
      ) : (
        <span
          className="thumb-sheet"
          style={{ aspectRatio: `${base?.width_pt ?? 1} / ${base?.height_pt ?? 1}` }}
        />
      )}
      <span
        className={`thumb-bookmark${bookmarked ? " on" : ""}`}
        role="button"
        tabIndex={0}
        title={bookmarked ? "Remove bookmark" : "Bookmark this page"}
        onPointerDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          e.stopPropagation();
          store.toggleBookmark(pageNo);
        }}>
        {bookmarked ? "★" : "☆"}
      </span>
      <span className="thumb-label">{pageNo}</span>
    </button>
  );
}

function PagesContent() {
  const app = useAppState();
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }));
  const sel = app.selection;
  const setSel = (next: number[] | ((s: number[]) => number[])) =>
    store.setPageSelection(typeof next === "function" ? next(app.selection) : next);
  const [anchor, setAnchor] = useState<number | null>(null);
  // Optimistic order while a drag is being applied; dropped on any doc change.
  const [pendingOrder, setPendingOrder] = useState<number[] | null>(null);
  const docOrder = app.doc?.pages.map((p) => p.page_no) ?? [];
  const order = pendingOrder ?? docOrder;
  const count = docOrder.length;

  useEffect(() => {
    setPendingOrder(null);
    setAnchor(null);
  }, [app.doc?.pages, app.doc?.path]);

  // Keep the current page's thumbnail visible.
  useEffect(() => {
    document
      .querySelector<HTMLElement>(`.thumb-list [data-thumb="${app.currentPage}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [app.currentPage]);

  function onDragEnd(e: DragEndEvent) {
    const { active, over } = e;
    if (!over || active.id === over.id) return;
    const next = arrayMove(order, order.indexOf(Number(active.id)), order.indexOf(Number(over.id)));
    setPendingOrder(next);
    void store.reorder(next).then((ok) => !ok && setPendingOrder(null));
  }

  function pick(pageNo: number, e: MouseEvent) {
    if (e.shiftKey && anchor !== null) {
      const [a, b] = anchor < pageNo ? [anchor, pageNo] : [pageNo, anchor];
      setSel(Array.from({ length: b - a + 1 }, (_, i) => a + i));
    } else if (e.metaKey || e.ctrlKey) {
      setSel((s) => {
        const base = s.length ? s : [app.currentPage];
        return base.includes(pageNo) ? base.filter((p) => p !== pageNo) : [...base, pageNo].sort((x, y) => x - y);
      });
      setAnchor(pageNo);
    } else {
      // Plain click selects just this page (Finder-style).
      setSel([pageNo]);
      setAnchor(pageNo);
    }
    store.goToPage(pageNo);
  }

  if (!app.doc) return <div className="pages-empty">No pages</div>;

  const locked = app.doc.passwordRequired || app.busy;
  const targets = sel.length ? sel : [app.currentPage];
  const what = sel.length > 1 ? `${sel.length} selected pages` : `page ${targets[0]}`;

  return (
    <>
      <div className="pages-tools" role="toolbar" aria-label="Page tools">
        <button onClick={() => void store.rotate(targets, false)} disabled={locked} title={`Rotate ${what} 90° counter-clockwise`} aria-label="Rotate left">
          <Icon name="rotate-ccw" size={16} />
        </button>
        <button onClick={() => void store.rotate(targets, true)} disabled={locked} title={`Rotate ${what} 90° clockwise`} aria-label="Rotate right">
          <Icon name="rotate-cw" size={16} />
        </button>
        <button
          onClick={() => void store.deletePages(targets)}
          disabled={locked || targets.length >= count}
          title={targets.length >= count ? "A document must keep at least one page" : `Delete ${what}`}
          aria-label="Delete pages">
          <Icon name="trash" size={16} />
        </button>
        <button
          onClick={() => void store.insertBlank(targets[targets.length - 1])}
          disabled={locked}
          title={`Insert a blank page after page ${targets[targets.length - 1]} (same size)`}
          aria-label="Insert blank page">
          <Icon name="insert-blank" size={16} />
        </button>
        <button onClick={() => void extractTo(targets)} disabled={locked} title={`Save ${what} as a new PDF`} aria-label="Extract pages">
          <Icon name="extract" size={16} />
        </button>
        {sel.length > 1 && (
          <button onClick={() => setSel([])} title="Clear selection" className="pages-sel">
            {sel.length} ✓
          </button>
        )}
      </div>
      <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
        <SortableContext items={order} strategy={verticalListSortingStrategy}>
          <div className="thumb-list">
            {order.map((pageNo) => (
              <div key={pageNo} data-thumb={pageNo}>
                <Thumb pageNo={pageNo} picked={sel.length > 1 && sel.includes(pageNo)} onPick={pick} />
              </div>
            ))}
          </div>
        </SortableContext>
      </DndContext>
      <div className="pages-hint">Drag to reorder · ⌘/Shift-click to select · ☆ to bookmark</div>
    </>
  );
}

function BookmarksContent() {
  const app = useAppState();
  const bookmarks = app.doc?.bookmarks ?? [];

  return (
    <>
      <div className="pages-tools">
        <button onClick={() => void store.addBookmark()} title="Add the current page">
          ＋ Page {app.currentPage}
        </button>
        <button
          onClick={() => void store.clearBookmarks()}
          disabled={bookmarks.length === 0}
          title="Remove all bookmarks">
          Clear
        </button>
      </div>
      <div className="thumb-list">
        {bookmarks.length === 0 && (
          <div className="pages-empty">
            No bookmarks yet — click ⭐ on a thumbnail to bookmark a page.
          </div>
        )}
        {bookmarks.map((n) => {
          const base = app.doc?.pages.find((p) => p.page_no === n);
          return (
            <div className="bm-row" key={`bm-${n}`}>
              {app.session && (
                <span className="bm-preview">
                  <PagePreview
                    session={app.session}
                    pageNo={n}
                    width={90}
                    rotation={base?.rotation ?? 0}
                  />
                </span>
              )}
              <button className="bm-jump" onClick={() => store.goToPage(n)} title={`Go to page ${n}`}>
                Page {n}
              </button>
              <button
                className="bm-del"
                onClick={() => void store.removeBookmark(n)}
                title="Remove bookmark">
                ✕
              </button>
            </div>
          );
        })}
      </div>
      <div className="pages-hint">Bookmarks live in this session only (never written to the PDF)</div>
    </>
  );
}

export function PageTree() {
  const app = useAppState();
  const tab = app.paneTab;
  const count = app.doc?.bookmarks.length ?? 0;

  return (
    <div className="pages-tree">
      <div className="pane-tabs" role="tablist" aria-label="Sidebar">
        <button
          role="tab"
          aria-selected={tab === "pages"}
          className={`pane-tab${tab === "pages" ? " active" : ""}`}
          onClick={() => store.setPaneTab("pages")}>
          Pages
        </button>
        <button
          role="tab"
          aria-selected={tab === "bookmarks"}
          className={`pane-tab${tab === "bookmarks" ? " active" : ""}`}
          onClick={() => store.setPaneTab("bookmarks")}>
          Bookmarks{count > 0 ? ` (${count})` : ""}
        </button>
        <button className="pane-close" onClick={() => store.setPaneOpen(false)} title="Close the sidebar (F8 shows it again)" aria-label="Close sidebar">
          ×
        </button>
      </div>
      {tab === "pages" ? <PagesContent /> : <BookmarksContent />}
    </div>
  );
}
