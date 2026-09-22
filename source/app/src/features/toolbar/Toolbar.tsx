import { open } from "@tauri-apps/plugin-dialog";
import { useAppState, store } from "../../store";
import { applyTheme, currentTheme, THEMES } from "../../themes/theme";

export function Toolbar() {
  const app = useAppState();

  async function pickAndOpen() {
    const chosen = await open({
      multiple: false,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (typeof chosen === "string") {
      await store.open(chosen);
    }
  }

  return (
    <header className="toolbar" role="toolbar" aria-label="main toolbar">
      <button onClick={pickAndOpen} disabled={app.busy}>
        Open…
      </button>
      <span className="divider" />
      <button onClick={() => void store.undo()} disabled={!app.doc || app.busy}>
        Undo
      </button>
      <button onClick={() => store.redo()} disabled={!app.doc || app.busy}>
        Redo
      </button>
      <span className="divider" />
      <button
        onClick={() => store.setZoom(app.zoom, "actual")}
        disabled={!app.doc}
        title="Actual size">
        Actual
      </button>
      <button
        onClick={() => store.setZoom(app.zoom, "fit-page")}
        disabled={!app.doc}
        title="Fit page">
        Fit
      </button>
      <button
        onClick={() => store.setZoom(Math.max(0.25, app.zoom - 0.1), "custom")}
        disabled={!app.doc}>
        −
      </button>
      <button
        onClick={() => store.setZoom(Math.min(5, app.zoom + 0.1), "custom")}
        disabled={!app.doc}>
        +
      </button>
      <span className="divider" />
      <button onClick={() => store.setMetadataOpen(true)} disabled={!app.doc}>
        Metadata…
      </button>
      <span className="divider" />
      <button
        onClick={() => store.setEditMode(!app.editMode)}
        disabled={!app.doc || app.doc.passwordRequired || app.doc.encrypted}
        className={app.editMode ? "active" : undefined}
        title="Edit text: click a text line in the document to replace it">
        Edit text
      </button>
      <span className="divider" />
      <select
        aria-label="Page view mode"
        value={app.viewMode}
        onChange={(e) => store.setViewMode(e.target.value as typeof app.viewMode)}
        disabled={!app.doc}
        title="Single page, side-by-side spread, or continuous scroll">
        <option value="single">Single</option>
        <option value="spread">Spread</option>
        <option value="continuous">Continuous</option>
      </select>
      <button
        onClick={() => store.setPaneOpen(!app.paneOpen)}
        disabled={!app.doc}
        title={app.paneOpen ? "Hide the sidebar (⌘⇧P)" : "Show the sidebar (⌘⇧P)"}>
        {app.paneOpen ? "Sidebar ›" : "‹ Sidebar"}
      </button>
      <span className="divider" />
      <select
        aria-label="Theme"
        defaultValue={currentTheme()}
        onChange={(e) => void applyTheme(e.target.value as (typeof THEMES)[number]["name"])}>
        {THEMES.map((t) => (
          <option key={t.name} value={t.name}>
            {t.label}
          </option>
        ))}
      </select>
    </header>
  );
}