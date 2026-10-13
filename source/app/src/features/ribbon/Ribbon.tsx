//! The Stitchup ribbon: a tab strip over groups of related controls, the way a
//! document app's chrome is organised (tabs -> labelled groups -> controls).
//! Independently designed for Stitchup; every control dispatches to the store.

import { useEffect, useState, type ReactNode } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useAppState, store } from "../../store";
import { applyTheme, currentTheme, THEMES, type ThemeName } from "../../themes/theme";
import { Icon, type IconName } from "./icons";

type TabId = "File" | "Home" | "Pages" | "Insert" | "View";

const TABS: TabId[] = ["File", "Home", "Pages", "Insert", "View"];

const SHORT_THEME: Record<ThemeName, string> = { xp: "Classic", dark: "Dark", blue: "Blue" };

/** A labelled group of controls, with a divider to its right. */
function Group({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section className="ribbon-group" aria-label={label}>
      <div className="ribbon-group-items">{children}</div>
      <div className="ribbon-group-label">{label}</div>
    </section>
  );
}

interface BtnProps {
  icon: IconName;
  label?: string;
  onClick: () => void;
  disabled?: boolean;
  active?: boolean;
  title?: string;
}

/** A large control: 24 px icon above a label. */
function Big({ icon, label, onClick, disabled, active, title }: BtnProps) {
  return (
    <button
      type="button"
      className={`rb-big${active ? " active" : ""}`}
      onClick={onClick}
      disabled={disabled}
      aria-pressed={active}
      title={title ?? label}>
      <span className="rb-icon">
        <Icon name={icon} size={24} />
      </span>
      {label && <span className="rb-text">{label}</span>}
    </button>
  );
}

/** A compact control: 16 px icon beside an optional label. */
function Small({ icon, label, onClick, disabled, active, title }: BtnProps) {
  return (
    <button
      type="button"
      className={`rb-small${active ? " active" : ""}`}
      onClick={onClick}
      disabled={disabled}
      aria-pressed={active}
      title={title ?? label}>
      <Icon name={icon} size={16} />
      {label && <span className="rb-text">{label}</span>}
    </button>
  );
}

export function Ribbon() {
  const app = useAppState();
  const [tab, setTab] = useState<TabId>("Home");
  const [backstage, setBackstage] = useState(false);
  const [goTo, setGoTo] = useState("");
  const [theme, setTheme] = useState<ThemeName>(currentTheme());

  const doc = app.doc;
  const busy = app.busy;
  const pageCount = doc?.pages.length ?? 0;
  const locked = !doc || doc.passwordRequired;
  const editable = !!doc && !doc.passwordRequired && !doc.encrypted;
  const canUndo = !locked && !busy && (doc?.history.length ?? 0) > 0;
  const canRedo = !locked && !busy && (doc?.redo.length ?? 0) > 0;
  const onPage = app.currentPage;

  useEffect(() => {
    if (!backstage) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setBackstage(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [backstage]);

  async function pickAndOpen() {
    const chosen = await openDialog({
      multiple: false,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (typeof chosen === "string") {
      setBackstage(false);
      await store.open(chosen);
    }
  }

  async function saveAs() {
    const picked = await saveDialog({
      defaultPath: doc ? `stitchup-export-from-${doc.sourceHash.slice(0, 8)}.pdf` : undefined,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (typeof picked === "string") {
      try {
        await store.commit(picked, false);
        setBackstage(false);
      } catch {
        // Surfaced through app.lastError / the status bar.
      }
    }
  }

  function chooseTheme(name: ThemeName) {
    setTheme(name);
    void applyTheme(name);
  }

  function selectTab(id: TabId) {
    if (id === "File") {
      setBackstage((open) => !open);
      return;
    }
    setBackstage(false);
    setTab(id);
  }

  function submitGoTo() {
    const n = Number(goTo);
    if (Number.isInteger(n) && n >= 1 && n <= pageCount) {
      store.goToPage(n);
      setGoTo("");
    }
  }

  return (
    <header className="ribbon" aria-label="ribbon toolbar">
      <div className="ribbon-tabs" role="tablist" aria-label="Ribbon tabs">
        {TABS.map((id) => {
          const selected = id === "File" ? backstage : !backstage && tab === id;
          return (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={selected}
              className={`ribbon-tab${selected ? " active" : ""}${id === "File" ? " file" : ""}`}
              onClick={() => selectTab(id)}>
              {id}
            </button>
          );
        })}
        <div className="ribbon-qat" aria-label="Quick access">
          <Small icon="open" onClick={pickAndOpen} disabled={busy} title="Open a PDF (⌘O)" />
          <Small icon="save" onClick={saveAs} disabled={locked || busy} title="Save As… (⌘⇧S)" />
          <span className="qat-sep" />
          <Small
            icon="undo"
            onClick={() => void store.undo()}
            disabled={!canUndo}
            title="Undo (⌘Z)"
          />
          <Small
            icon="redo"
            onClick={() => void store.redo()}
            disabled={!canRedo}
            title="Redo (⌘⇧Z)"
          />
        </div>
      </div>

      {backstage ? (
        <div className="ribbon-backstage">
          <nav className="backstage-nav" aria-label="File commands">
            <button type="button" className="backstage-item" onClick={pickAndOpen} disabled={busy}>
              <Icon name="open" size={20} /> Open…
            </button>
            <button
              type="button"
              className="backstage-item"
              onClick={saveAs}
              disabled={locked || busy}>
              <Icon name="save" size={20} /> Save As…
            </button>
            <button
              type="button"
              className="backstage-item"
              onClick={() => {
                store.setMetadataOpen(true);
                setBackstage(false);
              }}
              disabled={!doc}>
              <Icon name="info" size={20} /> Document Properties…
            </button>
          </nav>
          <div className="backstage-side">
            <h3>Stitchup</h3>
            <p className="muted">Offline-first PDF viewer &amp; editor</p>
            {doc && (
              <p className="muted" title={doc.path}>
                {pageCount} page{pageCount === 1 ? "" : "s"} · opened from source
              </p>
            )}
            <label className="backstage-field">
              <span>Theme</span>
              <select
                aria-label="Theme"
                value={theme}
                onChange={(e) => chooseTheme(e.target.value as ThemeName)}>
                {THEMES.map((t) => (
                  <option key={t.name} value={t.name}>
                    {t.label}
                  </option>
                ))}
              </select>
            </label>
          </div>
        </div>
      ) : (
        <div className="ribbon-body" role="tabpanel">
          {tab === "Home" && (
            <>
              <Group label="Clipboard">
                <Big
                  icon="copy"
                  label="Copy"
                  onClick={() => void store.copySelection()}
                  disabled={locked || busy || app.selectionText.length === 0}
                  title="Copy the selected text to the clipboard (⌘C)"
                />
              </Group>
              <Group label="File">
                <Big icon="open" label="Open" onClick={pickAndOpen} disabled={busy} title="Open a PDF (⌘O)" />
                <Big
                  icon="save"
                  label="Save As"
                  onClick={saveAs}
                  disabled={locked || busy}
                  title="Atomic save-as to a new file (⌘⇧S)"
                />
              </Group>
              <Group label="History">
                <Big icon="undo" label="Undo" onClick={() => void store.undo()} disabled={!canUndo} title="Undo the last change (⌘Z)" />
                <Big icon="redo" label="Redo" onClick={() => void store.redo()} disabled={!canRedo} title="Redo (⌘⇧Z)" />
              </Group>
              <Group label="Page">
                <div className="rb-grid">
                  <Small icon="rotate-cw" label="Rotate CW" onClick={() => void store.rotate([onPage], true)} disabled={locked || busy} title="Rotate this page 90° clockwise" />
                  <Small icon="rotate-ccw" label="Rotate CCW" onClick={() => void store.rotate([onPage], false)} disabled={locked || busy} title="Rotate this page 90° counter-clockwise" />
                  <Small icon="trash" label="Delete" onClick={() => void store.deletePages([onPage])} disabled={locked || busy} title="Delete this page" />
                  <Small icon="insert-blank" label="Blank page" onClick={() => void store.insertBlank(onPage, 612, 792)} disabled={locked || busy} title="Insert a US-Letter blank page after this one" />
                </div>
              </Group>
              <Group label="Edit">
                <Big
                  icon="text"
                  label="Edit Text"
                  active={app.editMode}
                  onClick={() => store.setEditMode(!app.editMode)}
                  disabled={!editable || busy}
                  title="Click a text line in the page to replace it"
                />
              </Group>
            </>
          )}

          {tab === "Pages" && (
            <>
              <Group label="Organise">
                <Big icon="rotate-cw" label="Rotate CW" onClick={() => void store.rotate([onPage], true)} disabled={locked || busy} title="Rotate the current page clockwise" />
                <Big icon="rotate-ccw" label="Rotate CCW" onClick={() => void store.rotate([onPage], false)} disabled={locked || busy} title="Rotate the current page counter-clockwise" />
                <Big icon="insert-blank" label="Insert Blank" onClick={() => void store.insertBlank(onPage, 612, 792)} disabled={locked || busy} title="Insert a blank page after the current page" />
                <Big icon="trash" label="Delete" onClick={() => void store.deletePages([onPage])} disabled={locked || busy} title="Delete the current page" />
              </Group>
              <Group label="Navigate">
                <div className="rb-nav">
                  <Small
                    icon="prev"
                    onClick={() => store.goToPage(onPage - 1)}
                    disabled={locked || onPage <= 1}
                    title="Previous page"
                  />
                  <input
                    className="rb-page"
                    inputMode="numeric"
                    aria-label="Go to page"
                    placeholder={`${onPage}`}
                    value={goTo}
                    onChange={(e) => setGoTo(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && submitGoTo()}
                    disabled={locked}
                  />
                  <span className="rb-muted">/ {pageCount || "—"}</span>
                  <Small
                    icon="next"
                    onClick={() => store.goToPage(onPage + 1)}
                    disabled={locked || onPage >= pageCount}
                    title="Next page"
                  />
                </div>
                <Small
                  icon="text"
                  label="Go"
                  onClick={submitGoTo}
                  disabled={locked || goTo.trim() === ""}
                  title="Jump to the typed page number"
                />
              </Group>
              <Group label="Bookmarks">
                <Big
                  icon="bookmark-add"
                  label="Add"
                  onClick={() => void store.addBookmark()}
                  disabled={locked}
                  title={`Bookmark page ${onPage}`}
                />
                <Big
                  icon="trash"
                  label="Clear All"
                  onClick={() => void store.clearBookmarks()}
                  disabled={locked || (doc?.bookmarks.length ?? 0) === 0}
                  title="Remove every bookmark (current session only)"
                />
              </Group>
            </>
          )}

          {tab === "Insert" && (
            <>
              <Group label="Pages">
                <Big icon="insert-blank" label="Blank Page" onClick={() => void store.insertBlank(onPage, 612, 792)} disabled={locked || busy} title="Insert a blank page after the current page" />
                <Big icon="merge" label="Merge PDF" onClick={() => void pickAndMerge()} disabled={locked || busy} title="Append the pages of another PDF" />
              </Group>
              <Group label="Text">
                <Big
                  icon="text"
                  label="Edit Text"
                  active={app.editMode}
                  onClick={() => store.setEditMode(!app.editMode)}
                  disabled={!editable || busy}
                  title="Click a text line in the page to replace it"
                />
              </Group>
              <Group label="Properties">
                <Big icon="info" label="Document" onClick={() => store.setMetadataOpen(true)} disabled={!doc} title="Title, author, subject, keywords" />
              </Group>
            </>
          )}

          {tab === "View" && (
            <>
              <Group label="Views">
                <Big
                  icon="single"
                  label="Single"
                  active={app.viewMode === "single"}
                  onClick={() => store.setViewMode("single")}
                  disabled={!doc}
                />
                <Big
                  icon="spread"
                  label="Spread"
                  active={app.viewMode === "spread"}
                  onClick={() => store.setViewMode("spread")}
                  disabled={!doc}
                />
                <Big
                  icon="continuous"
                  label="Continuous"
                  active={app.viewMode === "continuous"}
                  onClick={() => store.setViewMode("continuous")}
                  disabled={!doc}
                />
              </Group>
              <Group label="Zoom">
                <div className="rb-grid">
                  <Small icon="zoom-out" label="Zoom −" onClick={() => store.setZoom(Math.max(0.25, app.zoom - 0.1), "custom")} disabled={!doc} />
                  <Small icon="zoom-in" label="Zoom +" onClick={() => store.setZoom(Math.min(5, app.zoom + 0.1), "custom")} disabled={!doc} />
                  <Small icon="fit-page" label="Fit page" onClick={() => store.setZoom(app.zoom, "fit-page")} disabled={!doc} active={app.zoomMode === "fit-page"} />
                  <Small icon="fit-width" label="Fit width" onClick={() => store.setZoom(app.zoom, "fit-width")} disabled={!doc} active={app.zoomMode === "fit-width"} />
                  <Small icon="actual" label="Actual (1:1)" onClick={() => store.setZoom(1, "actual")} disabled={!doc} active={app.zoomMode === "actual"} />
                </div>
                <span className="ribbon-readout">{Math.round(app.zoom * 100)}%</span>
              </Group>
              <Group label="Show">
                <Small
                  icon="sidebar"
                  label={app.paneOpen ? "Hide Sidebar" : "Show Sidebar"}
                  active={app.paneOpen}
                  onClick={() => store.setPaneOpen(!app.paneOpen)}
                  disabled={!doc}
                  title="Toggle the sidebar (⌘⇧P)"
                />
                <Small
                  icon="thumbnails"
                  label="Thumbnails"
                  active={app.paneOpen && app.paneTab === "pages"}
                  onClick={() => store.setPaneTab("pages")}
                  disabled={!doc}
                />
                <Small
                  icon="bookmark"
                  label="Bookmarks"
                  active={app.paneOpen && app.paneTab === "bookmarks"}
                  onClick={() => store.setPaneTab("bookmarks")}
                  disabled={!doc}
                />
              </Group>
              <Group label="Theme">
                {(Object.keys(SHORT_THEME) as ThemeName[]).map((name) => (
                  <Big
                    key={name}
                    icon="palette"
                    label={SHORT_THEME[name]}
                    active={theme === name}
                    onClick={() => chooseTheme(name)}
                    title={`${SHORT_THEME[name]} theme`}
                  />
                ))}
              </Group>
            </>
          )}
        </div>
      )}
    </header>
  );

  async function pickAndMerge() {
    const chosen = await openDialog({
      multiple: false,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (typeof chosen === "string") {
      await store.merge(chosen);
    }
  }
}
