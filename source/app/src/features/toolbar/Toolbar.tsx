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

  async function pickAndMerge() {
    const chosen = await open({
      multiple: false,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (typeof chosen === "string") {
      await store.merge(chosen);
    }
  }

  const locked = !app.doc || app.doc.passwordRequired;

  return (
    <header className="toolbar" role="toolbar" aria-label="main toolbar">
      <button onClick={pickAndOpen} disabled={app.busy}>
        Open…
      </button>
      {app.doc?.passwordRequired && (
        <button onClick={() => store.setUnlockOpen(true)} disabled={app.busy} className="warn">
          Unlock…
        </button>
      )}
      <span className="divider" />
      <button
        onClick={() => void store.undo()}
        disabled={locked || app.busy || app.doc?.history.length === 0}
        title="Undo the last structural change (⌘Z)">
        Undo
      </button>
      <button
        onClick={() => void store.redo()}
        disabled={locked || app.busy || app.doc?.redo.length === 0}
        title="Redo (⌘⇧Z)">
        Redo
      </button>
      <span className="divider" />
      <button
        onClick={pickAndMerge}
        disabled={locked || app.busy}
        title="Append the pages of another PDF to this document">
        Merge…
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