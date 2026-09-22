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
import { useState } from "react";
import { useAppState, store } from "../../store";

function Thumb({ pageNo }: { pageNo: number }) {
  const app = useAppState();
  const { attributes, listeners, setNodeRef, transform, transition } = useSortable({
    id: pageNo,
  });
  const base = app.doc?.pages.find((p) => p.page_no === pageNo);
  const selected = app.currentPage === pageNo;

  return (
    <button
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      {...attributes}
      {...listeners}
      className={`thumb${selected ? " selected" : ""}`}
      onClick={() => store.goToPage(pageNo)}
      title={`Page ${pageNo} — rotation ${base?.rotation ?? 0}°`}
    >
      <span
        className="thumb-sheet"
        style={{ aspectRatio: `${base?.width_pt ?? 1} / ${base?.height_pt ?? 1}` }}
      />
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
      <div className="pages-hint">Drag to reorder · right-click thumb to toggle selection</div>
    </>
  );

  function toggle(pageNo: number) {
    setSel((s) => (s.includes(pageNo) ? s.filter((p) => p !== pageNo) : [...s, pageNo]));
  }
}

// Bookmark jump button needs the page navigation from the store only.
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
            No bookmarks yet — open a document and use “＋ Page N”.
          </div>
        )}
        {bookmarks.map((n) => (
          <div className="bm-row" key={`bm-${n}`}>
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
        ))}
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