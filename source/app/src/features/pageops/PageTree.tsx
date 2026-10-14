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
import { useEffect, useRef, useState } from "react";
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

function Thumb({ pageNo }: { pageNo: number }) {
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
      className={`thumb${selected ? " selected" : ""}${bookmarked ? " bookmarked" : ""}`}
      onClick={() => store.goToPage(pageNo)}
      title={`Page ${pageNo} — rotation ${base?.rotation ?? 0}° · double-click ⭐ to bookmark`}
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
  const [items, setItems] = useState<number[]>([]);
  const [sel, setSel] = useState<number[]>([]);

  // Keep a local order mirror for drag; it is authoritative for dnd-kit only.
  const order = items.length ? items : app.doc?.pages.map((p) => p.page_no) ?? [];

  function onDragEnd(e: DragEndEvent) {
    const { active, over } = e;
    if (!over || active.id === over.id) {
      return;
    }
    const oldIndex = order.indexOf(Number(active.id));
    const newIndex = order.indexOf(Number(over.id));
    const next = arrayMove(order, oldIndex, newIndex);
    setItems(next);
    void store.reorder(next);
  }

  if (!app.doc) {
    return <div className="pages-empty">No pages</div>;
  }

  return (
    <>
      <div className="pages-tools">
        <button
          onClick={() => void store.rotate(sel, true)}
          disabled={sel.length === 0}
          title="Rotate selected 90° CW">
          ⟳
        </button>
        <button
          onClick={() => void store.deletePages(sel)}
          disabled={sel.length === 0}
          title="Delete selected">
          ✕
        </button>
        <button onClick={() => void store.insertBlank(sel[0] ?? 0, 612, 792)} title="Insert blank after selection">
          +
        </button>
      </div>
      <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
        <SortableContext items={order} strategy={verticalListSortingStrategy}>
          <div className="thumb-list">
            {order.map((pageNo) => (
              <div key={pageNo} onContextMenu={() => toggle(pageNo)}>
                <Thumb pageNo={pageNo} />
              </div>
            ))}
          </div>
        </SortableContext>
      </DndContext>
      <div className="pages-hint">Drag to reorder · click ⭐ to bookmark · right-click to select</div>
    </>
  );

  function toggle(pageNo: number) {
    setSel((s) => (s.includes(pageNo) ? s.filter((p) => p !== pageNo) : [...s, pageNo]));
  }
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
      </div>
      {tab === "pages" ? <PagesContent /> : <BookmarksContent />}
    </div>
  );
}
