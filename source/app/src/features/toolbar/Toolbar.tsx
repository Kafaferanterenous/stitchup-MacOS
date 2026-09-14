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
        onClick={() => store.setViewMode(app.viewMode === "continuous" ? "single" : "continuous")}
        disabled={!app.doc}
        title="Toggle single/continuous page view">
        View
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