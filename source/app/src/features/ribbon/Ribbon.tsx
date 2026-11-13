//! The Stitchup ribbon — same layout as the Windows build: a brand accent
//! strip, the tab strip (Document · Pages · Annotate · Content · Zoom ·
//! Navigate · Security) and one card of icon-over-caption buttons. Every
//! button comes from the command registry (src/commands.ts).

import { useEffect, useRef, useState } from "react";
import { useAppState, store, RIBBON_TABS } from "../../store";
import { COMMANDS, runCommand, type Command } from "../../commands";
import { Icon } from "./icons";

function RibbonButton({ cmd }: { cmd: Command }) {
  const app = useAppState();
  const [open, setOpen] = useState(false);
  const colorRef = useRef<HTMLInputElement | null>(null);
  const enabled = !app.busy && (!cmd.enabled || cmd.enabled(app));
  const active = cmd.active?.(app) ?? false;
  const dirty = cmd.id === "save" && store.isDirty();

  // Menu bar / shortcut asked to show this button's drop-down or colour picker.
  useEffect(() => {
    const onChoices = (e: Event) => (e as CustomEvent).detail === cmd.id && setOpen(true);
    const onSwatch = (e: Event) => cmd.swatch && (e as CustomEvent).detail === cmd.swatch && cmd.tab === app.ribbonTab && colorRef.current?.click();
    window.addEventListener("stitchup-choices", onChoices);
    window.addEventListener("stitchup-swatch", onSwatch);
    return () => {
      window.removeEventListener("stitchup-choices", onChoices);
      window.removeEventListener("stitchup-swatch", onSwatch);
    };
  }, [cmd.id, cmd.swatch, cmd.tab, app.ribbonTab]);

  useEffect(() => {
    if (!open) return;
    const close = (e: Event) => {
      if (e instanceof KeyboardEvent && e.key !== "Escape") return;
      setOpen(false);
    };
    const t = setTimeout(() => window.addEventListener("mousedown", close), 0);
    window.addEventListener("keydown", close);
    return () => {
      clearTimeout(t);
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", close);
    };
  }, [open]);

  const swatchColor = cmd.swatch === "fore" ? app.foreColor : cmd.swatch === "back" ? app.backColor : null;

  return (
    <span className="rb-wrap">
      <button
        type="button"
        className={`rb-btn${active ? " active" : ""}${dirty ? " dirty" : ""}${cmd.choices ? " has-choices" : ""}`}
        onClick={() => {
          if (cmd.choices) setOpen((o) => !o);
          else if (cmd.swatch) colorRef.current?.click();
          else void runCommand(cmd.id);
        }}
        onContextMenu={(e) => {
          if (cmd.swatch === "back") {
            e.preventDefault();
            store.setColors({ back: null });
            store.notify("Back colour: no fill");
          }
        }}
        disabled={!enabled}
        aria-pressed={cmd.active ? active : undefined}
        aria-haspopup={cmd.choices ? "menu" : undefined}
        aria-expanded={cmd.choices ? open : undefined}
        data-cmd={cmd.id}
        title={dirty ? `${cmd.tip} — unsaved changes` : cmd.tip}>
        <span className="rb-icon">
          {cmd.swatch ? (
            <span className={`rb-swatch${swatchColor ? "" : " none"}`} style={{ background: swatchColor ?? undefined }} />
          ) : (
            <Icon name={cmd.icon} size={20} />
          )}
        </span>
        <span className="rb-text">
          {cmd.label}
          {cmd.choices ? " ▾" : ""}
        </span>
      </button>
      {cmd.swatch && (
        <input
          ref={colorRef}
          type="color"
          className="rb-color-input"
          aria-label={`${cmd.label} colour`}
          value={swatchColor ?? "#ffffff"}
          onChange={(e) => store.setColors(cmd.swatch === "fore" ? { fore: e.target.value } : { back: e.target.value })}
        />
      )}
      {open && cmd.choices && (
        <div className="rb-choices" role="menu" onMouseDown={(e) => e.stopPropagation()}>
          {cmd.choices().map((c) => (
            <button
              key={c.label}
              role="menuitem"
              className={c.active ? "active" : ""}
              onClick={() => {
                setOpen(false);
                c.run();
              }}>
              {c.label}
            </button>
          ))}
        </div>
      )}
    </span>
  );
}

/** Page box for the Navigate tab: type a number, Enter. */
function PageBox() {
  const app = useAppState();
  const [v, setV] = useState("");
  const count = app.doc?.pages.length ?? 0;
  function go() {
    const n = Number(v);
    if (Number.isInteger(n) && n >= 1 && n <= count) {
      store.goToPage(n);
      setV("");
    } else store.notify(`Enter a page number from 1 to ${count}`);
  }
  return (
    <div className="rb-pagebox">
      <input aria-label="Go to page" inputMode="numeric" placeholder={`${app.currentPage}`} value={v} onChange={(e) => setV(e.target.value)} onKeyDown={(e) => e.key === "Enter" && go()} disabled={!count} />
      <span>/ {count || "—"}</span>
      <button type="button" className="rb-go" onClick={go} disabled={!count || !v.trim()}>
        Go
      </button>
    </div>
  );
}

/** Find box on the tab row (⌘F focuses it; Enter / F3 / ⇧F3 step through). */
function SearchBox() {
  const app = useAppState();
  const hits = app.searchHits.length;
  return (
    <div className="search-box" role="search">
      <Icon name="find" size={14} />
      <input
        value={app.query}
        placeholder="Find"
        aria-label="Find text"
        disabled={!app.doc}
        onChange={(e) => store.setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") void (hits && app.query ? store.findNext(e.shiftKey ? -1 : 1) : store.find());
          if (e.key === "Escape") store.setQuery("");
        }}
      />
      <span className="search-status" aria-live="polite">
        {hits ? `${app.searchIndex + 1}/${hits}` : ""}
      </span>
      <button type="button" aria-label="Previous match" onClick={() => void store.findNext(-1)} disabled={!hits}>
        ‹
      </button>
      <button type="button" aria-label="Next match" onClick={() => void store.findNext(1)} disabled={!hits}>
        ›
      </button>
    </div>
  );
}

export function Ribbon() {
  const app = useAppState();
  const tab = app.ribbonTab;
  const buttons = COMMANDS.filter((c) => c.tab === tab);
  return (
    <header className="ribbon" aria-label="ribbon toolbar">
      <div className="ribbon-strip" />
      <div className="ribbon-tabs" role="tablist" aria-label="Ribbon tabs">
        {RIBBON_TABS.map((t) => (
          <button key={t} type="button" role="tab" aria-selected={t === tab} className={`ribbon-tab${t === tab ? " active" : ""}`} onClick={() => store.setRibbonTab(t)}>
            {t}
          </button>
        ))}
        <SearchBox />
      </div>
      <div className="ribbon-body" role="tabpanel" aria-label={tab}>
        <div className="ribbon-card">
          {buttons.map((c) => (
            <RibbonButton key={c.id} cmd={c} />
          ))}
          {tab === "Navigate" && <PageBox />}
          {tab === "Zoom" && <span className="ribbon-readout">{Math.round(app.zoom * 100)}%</span>}
        </div>
      </div>
    </header>
  );
}
