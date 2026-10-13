import { useEffect } from "react";
import { useAppState, store, SHORTCUTS, matchesShortcut } from "./store";
import type { ViewMode } from "./types";
import { Ribbon } from "./features/ribbon/Ribbon";
import { Viewer } from "./features/viewer/Viewer";
import { PageTree } from "./features/pageops/PageTree";
import { SearchBar } from "./features/search/SearchBar";
import { MetadataModal } from "./features/metadata/MetadataModal";
import { CommitPanel } from "./features/commit/CommitPanel";
import { MessageBox } from "./features/common/MessageBox";
import { PasswordDialog } from "./features/common/PasswordDialog";

const CYCLE_VIEW: ViewMode[] = ["single", "spread", "continuous"];

export default function App() {
  const app = useAppState();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (matchesShortcut(e, SHORTCUTS.open)) {
        e.preventDefault();
        void awaitPick().then((p) => {
          if (p) void store.open(p);
        });
      } else if (matchesShortcut(e, SHORTCUTS.undo)) {
        e.preventDefault();
        void store.undo();
      } else if (matchesShortcut(e, SHORTCUTS.redo)) {
        e.preventDefault();
        void store.redo();
      } else if (matchesShortcut(e, SHORTCUTS.zoomIn)) {
        e.preventDefault();
        store.setZoom(app.zoom * 1.1, "custom");
      } else if (matchesShortcut(e, SHORTCUTS.zoomOut)) {
        e.preventDefault();
        store.setZoom(app.zoom / 1.1, "custom");
      } else if (matchesShortcut(e, SHORTCUTS.fitPage)) {
        e.preventDefault();
        store.setZoom(app.zoom, "fit-page");
      } else if (matchesShortcut(e, SHORTCUTS.fitWidth)) {
        e.preventDefault();
        store.setZoom(app.zoom, "fit-width");
      } else if (matchesShortcut(e, SHORTCUTS.toggleView)) {
        e.preventDefault();
        const modes = CYCLE_VIEW;
        const next = modes[(modes.indexOf(app.viewMode) + 1) % modes.length];
        store.setViewMode(next);
      } else if (matchesShortcut(e, SHORTCUTS.togglePane)) {
        e.preventDefault();
        store.setPaneOpen(!app.paneOpen);
      } else if (e.key === "Escape" && app.editMode) {
        e.preventDefault();
        store.setEditMode(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [app.zoom, app.viewMode, app.paneOpen]);

  return (
    <div className="app-shell" data-view={app.viewMode}>
      <Ribbon />
      <SearchBar />
      <main className="app-body" data-pane={app.paneOpen ? "open" : "hidden"}>
        <section className="viewer-pane" aria-label="document viewer">
          <Viewer />
        </section>
        {app.paneOpen && (
          <aside className="pages-pane" aria-label="page thumbnails">
            <PageTree />
          </aside>
        )}
      </main>
      <footer className="status-bar" role="status">
        <span>
          {app.doc
            ? `Page ${app.currentPage}/${app.doc.pages.length} · zoom ${Math.round(app.zoom * 100)}%`
            : app.statusMessage}
        </span>
        <span className="status-right">
          {app.doc && app.doc.passwordRequired && <b className="warn">password-protected</b>}
          {app.doc && app.doc.encrypted && <small className="muted">encrypted</small>}
          {app.lastCommit && (
            <small className="muted" title={app.lastCommit.sha}>
              saved: {app.lastCommit.size} B
            </small>
          )}
        </span>
      </footer>
      <MetadataModal open={app.metadataOpen} />
      <PasswordDialog />
      <CommitPanel />
      <MessageBox />
      {app.notice && (
        <div className="notice-toast" role="status">
          {app.notice}
        </div>
      )}
    </div>
  );

  async function awaitPick(): Promise<string> {
    // Deferred: the real file dialog lives in the Ribbon/CommitPanel via the
    // dialog plugin when running under Tauri. In dev (browser) we type a path.
    return (
      window.prompt("Path to a PDF (Tauri file dialog lands here at runtime)", "") ?? ""
    );
  }
}