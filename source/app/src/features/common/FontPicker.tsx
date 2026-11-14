//! Font chooser: the line's original font, the three standard PDF fonts
//! (never embedded) and every installed font that may be embedded — each name
//! shown in its own typeface, with a search box.

import { useEffect, useMemo, useRef, useState } from "react";
import { cmdListSystemFonts, type SystemFace } from "../../ipc";

export const STANDARD = ["Helvetica", "Times", "Courier"] as const;
export type FontChoice = "original" | string; // "original", a standard family, or an installed family

let cache: Promise<SystemFace[]> | null = null;
export function loadSystemFonts(): Promise<SystemFace[]> {
  cache ??= cmdListSystemFonts().catch(() => {
    cache = null;
    return [];
  });
  return cache;
}

export function useSystemFonts(): SystemFace[] {
  const [faces, setFaces] = useState<SystemFace[]>([]);
  useEffect(() => {
    let alive = true;
    void loadSystemFonts().then((f) => alive && setFaces(f));
    return () => {
      alive = false;
    };
  }, []);
  return faces;
}

/** Standard-14 font name for a family + style. */
export function std14Name(family: string, bold: boolean, italic: boolean): string {
  if (family === "Times") return bold && italic ? "Times-BoldItalic" : bold ? "Times-Bold" : italic ? "Times-Italic" : "Times-Roman";
  const sfx = bold && italic ? "-BoldOblique" : bold ? "-Bold" : italic ? "-Oblique" : "";
  return family + sfx;
}

/** Face of an installed family closest to bold / italic. */
export function faceIn(faces: SystemFace[], family: string, bold: boolean, italic: boolean): SystemFace | undefined {
  const want = bold ? 700 : 400;
  const score = (f: SystemFace) => (f.italic !== italic ? 10000 : 0) + Math.abs(f.weight - want) + (/condensed|narrow|compressed/i.test(f.style) ? 500 : 0);
  return faces.filter((f) => f.family === family).sort((a, b) => score(a) - score(b))[0];
}

/** The font name to send to the backend ("" = keep the original font). */
export function fontName(faces: SystemFace[], choice: FontChoice, bold: boolean, italic: boolean): string {
  if (choice === "original") return "";
  if ((STANDARD as readonly string[]).includes(choice)) return std14Name(choice, bold, italic);
  return faceIn(faces, choice, bold, italic)?.postscript ?? std14Name("Helvetica", bold, italic);
}

const norm = (s: string) => s.toLowerCase().replace(/^[a-z]{6}\+/, "").replace(/[\s,_-]|ps|mt$/g, "");

/** Installed family a PDF font name belongs to (e.g. "ABCDEF+Arial-BoldMT" → "Arial"). */
export function familyOf(faces: SystemFace[], pdfFont: string): string | null {
  const n = norm(pdfFont);
  const exact = faces.find((f) => norm(f.postscript) === n);
  if (exact) return exact.family;
  const base = norm(pdfFont.replace(/^[A-Z]{6}\+/, "").split(/[-,]/)[0]);
  const fam = faces.find((f) => norm(f.family) === base || norm(f.postscript.split("-")[0]) === base);
  return fam?.family ?? null;
}

export function FontPicker({
  value,
  onPick,
  allowOriginal,
  originalLabel = "Original font",
}: {
  value: FontChoice;
  onPick: (c: FontChoice) => void;
  allowOriginal?: boolean;
  originalLabel?: string;
}) {
  const faces = useSystemFonts();
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const ref = useRef<HTMLSpanElement | null>(null);
  const families = useMemo(() => {
    const m = new Map<string, SystemFace[]>();
    for (const f of faces) m.set(f.family, [...(m.get(f.family) ?? []), f]);
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [faces]);
  const needle = q.trim().toLowerCase();
  const shown = families.filter(([fam]) => !needle || fam.toLowerCase().includes(needle));

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  const pick = (c: FontChoice) => {
    setOpen(false);
    setQ("");
    onPick(c);
  };
  const label = value === "original" ? originalLabel : value;
  return (
    <span className="font-picker" ref={ref}>
      <button
        type="button"
        className="font-picker-button"
        aria-label="Font"
        aria-haspopup="listbox"
        aria-expanded={open}
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => setOpen(!open)}
        style={{ fontFamily: value === "original" ? undefined : `"${value === "Times" ? "Times New Roman" : value}", sans-serif` }}>
        {label} ▾
      </button>
      {open && (
        <div className="font-picker-pop" onMouseDown={(e) => e.stopPropagation()}>
          <input
            className="font-picker-search"
            placeholder={`Search ${families.length} installed fonts`}
            aria-label="Search fonts"
            value={q}
            autoFocus
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Escape") setOpen(false);
              if (e.key === "Enter" && shown.length) pick(shown[0][0]);
            }}
          />
          <div className="font-picker-list" role="listbox" aria-label="Fonts">
            {allowOriginal && !needle && (
              <button type="button" role="option" aria-selected={value === "original"} className={value === "original" ? "on" : ""} onClick={() => pick("original")}>
                {originalLabel}
              </button>
            )}
            {!needle && <div className="font-picker-head">Standard — not embedded, smallest file</div>}
            {!needle &&
              STANDARD.map((f) => (
                <button key={f} type="button" role="option" aria-selected={value === f} className={value === f ? "on" : ""} style={{ fontFamily: f === "Times" ? '"Times New Roman", serif' : f === "Courier" ? '"Courier New", monospace' : "Helvetica, sans-serif" }} onClick={() => pick(f)}>
                  {f}
                </button>
              ))}
            <div className="font-picker-head">{faces.length ? "Installed on this Mac — embedded" : "Loading installed fonts…"}</div>
            {shown.map(([fam, fs]) => (
              <button key={fam} type="button" role="option" aria-selected={value === fam} className={value === fam ? "on" : ""} onClick={() => pick(fam)} title={`${fam} — ${fs.length} style${fs.length > 1 ? "s" : ""}`}>
                <span style={{ fontFamily: `"${fam}", sans-serif` }}>{fam}</span>
                <span className="font-picker-tags">
                  {fs.some((f) => f.cyrillic) ? " Кир" : ""}
                  {fs.some((f) => f.greek) ? " Ελ" : ""}
                  {fs.some((f) => f.cjk) ? " 中" : ""}
                </span>
              </button>
            ))}
            {needle && !shown.length && <div className="font-picker-head">No font matches “{q}”</div>}
          </div>
        </div>
      )}
    </span>
  );
}
