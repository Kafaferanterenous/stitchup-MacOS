//! Document tabs (like the Windows build): click to switch, × to close
//! (asks first when unsaved), + for a new tab. Thumbnails dropped on a tab
//! move those pages to that document.

import { useAppState, store } from "../../store";

export function TabStrip() {
  const app = useAppState();
  if (!app.doc && app.tabs.length <= 1) return null;
  return (
    <nav className="tab-strip" role="tablist" aria-label="Open documents">
      {app.tabs.map((t) => (
        <div
          key={t.id}
          role="tab"
          aria-selected={t.id === app.activeTab}
          data-tab-id={t.id}
          className={`doc-tab${t.id === app.activeTab ? " active" : ""}`}
          title={t.path ?? t.name}
          onMouseDown={(e) => {
            if (e.button === 1) void store.closeTab(t.id); // middle-click closes
          }}
          onClick={() => void store.switchTab(t.id)}>
          <span className="doc-tab-name">
            {t.name}
            {t.dirty ? " *" : ""}
          </span>
          <button
            type="button"
            className="doc-tab-close"
            aria-label={`Close ${t.name}`}
            title="Close tab (⌘W)"
            onClick={(e) => {
              e.stopPropagation();
              void store.closeTab(t.id);
            }}>
            ×
          </button>
        </div>
      ))}
      <button type="button" className="doc-tab-new" aria-label="New tab" title="New tab (⌘T)" onClick={() => void store.newTab()}>
        +
      </button>
    </nav>
  );
}
