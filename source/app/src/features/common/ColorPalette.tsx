//! Colour palette pop-up (ribbon Fore / Back / Annotate ▸ Colour, and the
//! text edit bar). Our own grid — the hidden <input type="color"> trick does
//! not open a picker inside the macOS web view.

import { useState } from "react";

export const PALETTE = [
  "#000000", "#434343", "#666666", "#999999", "#cccccc", "#ffffff",
  "#d0021b", "#e8590c", "#f5a623", "#ffeb3b", "#7ed321", "#1a7f37",
  "#00a3a3", "#4a90e2", "#1d5fd0", "#0b2a6f", "#7c4dbe", "#c2185b",
  "#ffcdd2", "#ffe0b2", "#fff9c4", "#c8e6c9", "#bbdefb", "#e1bee7",
];

/** Keep focus in an open text editor: buttons here never take focus. */
const keepFocus = (e: React.MouseEvent) => e.preventDefault();

export function ColorPalette({
  value,
  onPick,
  allowNone,
  noneLabel = "No fill",
  label,
}: {
  value: string | null;
  onPick: (hex: string | null) => void;
  allowNone?: boolean;
  noneLabel?: string;
  label: string;
}) {
  const [hex, setHex] = useState(value ?? "#000000");
  const valid = /^#[0-9a-f]{6}$/i.test(hex);
  return (
    <div className="color-palette" role="menu" aria-label={label} onMouseDown={(e) => e.stopPropagation()}>
      <div className="color-grid">
        {PALETTE.map((c) => (
          <button
            key={c}
            type="button"
            role="menuitemradio"
            aria-checked={value?.toLowerCase() === c}
            aria-label={c}
            title={c}
            className={`color-cell${value?.toLowerCase() === c ? " on" : ""}`}
            style={{ background: c }}
            onMouseDown={keepFocus}
            onClick={() => onPick(c)}
          />
        ))}
      </div>
      {allowNone && (
        <button type="button" role="menuitem" className={`color-none${value === null ? " on" : ""}`} onMouseDown={keepFocus} onClick={() => onPick(null)}>
          {noneLabel}
        </button>
      )}
      <div className="color-custom">
        <span className="color-sample" style={{ background: valid ? hex : "transparent" }} />
        <input
          aria-label="Colour code"
          value={hex}
          spellCheck={false}
          onChange={(e) => setHex(e.target.value.trim())}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter" && valid) onPick(hex.toLowerCase());
          }}
        />
        <button type="button" disabled={!valid} onMouseDown={keepFocus} onClick={() => onPick(hex.toLowerCase())}>
          Use
        </button>
      </div>
    </div>
  );
}
