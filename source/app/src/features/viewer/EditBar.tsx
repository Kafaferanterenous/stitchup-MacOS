//! Format bar shown above a line while it is being edited: the line's real
//! font, size and colour, and controls to change them (applied to the whole
//! line; typed text is saved first). Any installed font can be chosen.

import { useEffect, useState } from "react";
import { store, rgb, type FormatOp } from "../../store";
import type { TextRun } from "../../types";
import { ColorPalette } from "../common/ColorPalette";
import { FontPicker, STANDARD, familyOf, fontName, useSystemFonts, type FontChoice } from "../common/FontPicker";

export { std14Name } from "../common/FontPicker";

/** PDF box around a run's baseline (for finding its content objects). */
export function runBox(run: TextRun, widthScale = 1): [number, number, number, number] {
  return [run.origin.x, run.origin.y - run.fontSize * 0.35, run.origin.x + run.pdfWidth * widthScale, run.origin.y + run.fontSize * 0.95];
}

const keepFocus = (e: React.MouseEvent) => e.preventDefault();

export function EditBar({ run, page, zoom }: { run: TextRun; page: number; zoom: number }) {
  const faces = useSystemFonts();
  const [info, setInfo] = useState<{ font: string; size: number; color: string } | null>(null);
  const [family, setFamily] = useState<FontChoice>("original");
  const [bold, setBold] = useState(false);
  const [italic, setItalic] = useState(false);
  const [size, setSize] = useState(String(Math.round(run.fontSize * 10) / 10));
  const [palette, setPalette] = useState(false);

  useEffect(() => {
    let alive = true;
    store.lineObjects(page, runBox(run)).then(
      (objs) => {
        const o = objs.find((x) => x.font) ?? objs[0];
        if (!alive || !o) return;
        const font = o.font ?? "";
        setInfo({ font, size: o.size ?? run.fontSize, color: o.color ?? "#000000" });
        setSize(String(o.size ?? Math.round(run.fontSize * 10) / 10));
        setBold(/bold|black|heavy|semibold|demi/i.test(font));
        setItalic(/italic|oblique/i.test(font));
      },
      () => {},
    );
    return () => {
      alive = false;
    };
  }, [page, run]);

  const apply = (d: FormatOp) => void store.objectOp(d);
  const sizeNum = Number(size);
  const sizeOk = Number.isFinite(sizeNum) && sizeNum >= 1 && sizeNum <= 400;
  const sizeChanged = sizeOk && Math.abs(sizeNum - (info?.size ?? run.fontSize)) > 0.05;

  /** Family for bold / italic of the ORIGINAL font: the same family if installed, else the closest standard one. */
  function originalFamily(): FontChoice {
    const n = info?.font ?? "";
    const std = STANDARD.find((s) => n.startsWith(s));
    if (std) return std; // standard PDF fonts stay standard (nothing embedded)
    const installed = familyOf(faces, n);
    if (installed) return installed;
    if (/courier|mono|menlo/i.test(n)) return "Courier";
    if (/times|serif|georgia|garamond|roman|book|minion|cambria/i.test(n) && !/sans/i.test(n)) return "Times";
    return "Helvetica";
  }

  function applyFont(f: FontChoice, b: boolean, i: boolean) {
    if (f === "original" && (b !== bold || i !== italic)) {
      f = originalFamily();
      setFamily(f);
    }
    if (f === "original" && !sizeChanged) return;
    apply({ op: "font", family: fontName(faces, f, b, i), size: sizeChanged ? sizeNum : undefined });
  }

  const top = run.rect.y * zoom - 40;
  return (
    <div
      className="edit-bar"
      role="toolbar"
      aria-label="Text format"
      style={{ left: Math.max(0, run.rect.x * zoom - 2), top: top < 0 ? (run.rect.y + run.rect.height) * zoom + 6 : top }}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}>
      <span className="edit-bar-font" title="The line's font in the PDF">
        {info ? `${info.font || "Unknown font"} · ${info.size} pt` : "…"}
      </span>
      <FontPicker
        value={family}
        allowOriginal
        onPick={(f) => {
          setFamily(f);
          applyFont(f, bold, italic);
        }}
      />
      <button
        type="button"
        className={`edit-bar-toggle${bold ? " on" : ""}`}
        aria-pressed={bold}
        aria-label="Bold"
        title="Bold"
        onMouseDown={keepFocus}
        onClick={() => {
          setBold(!bold);
          applyFont(family, !bold, italic);
        }}>
        <b>B</b>
      </button>
      <button
        type="button"
        className={`edit-bar-toggle${italic ? " on" : ""}`}
        aria-pressed={italic}
        aria-label="Italic"
        title="Italic"
        onMouseDown={keepFocus}
        onClick={() => {
          setItalic(!italic);
          applyFont(family, bold, !italic);
        }}>
        <i>I</i>
      </button>
      <input
        aria-label="Font size"
        className="edit-bar-size"
        inputMode="decimal"
        value={size}
        onChange={(e) => setSize(e.target.value)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter" && sizeOk) apply({ op: "font", family: fontName(faces, family, bold, italic), size: sizeNum });
        }}
      />
      <span className="edit-bar-unit">pt</span>
      <span className="edit-bar-color">
        <button type="button" aria-label="Text colour" title="Text colour" aria-haspopup="menu" aria-expanded={palette} onMouseDown={keepFocus} onClick={() => setPalette(!palette)}>
          <span className="rb-swatch" style={{ background: info?.color ?? "#000" }} />▾
        </button>
        {palette && (
          <div className="rb-choices rb-palette">
            <ColorPalette
              label="Text colour"
              value={info?.color ?? null}
              onPick={(c) => {
                setPalette(false);
                if (c) apply({ op: "recolor", color: rgb(c) });
              }}
            />
          </div>
        )}
      </span>
    </div>
  );
}
