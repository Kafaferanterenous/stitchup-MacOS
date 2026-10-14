//! Original, code-drawn icons for the ribbon (clean-room: no copied artwork).
//! 24x24 viewBox, stroke follows `currentColor` so themes just work.

import type { ReactNode } from "react";

export type IconName =
  | "open"
  | "save"
  | "undo"
  | "redo"
  | "rotate-cw"
  | "rotate-ccw"
  | "trash"
  | "copy"
  | "insert-blank"
  | "text"
  | "zoom-in"
  | "zoom-out"
  | "fit-page"
  | "fit-width"
  | "actual"
  | "prev"
  | "next"
  | "bookmark"
  | "bookmark-add"
  | "merge"
  | "info"
  | "single"
  | "spread"
  | "continuous"
  | "sidebar"
  | "thumbnails"
  | "palette"
  | "exit";

const ICONS: Record<IconName, ReactNode> = {
  open: (
    <path d="M3 6.5A1.5 1.5 0 0 1 4.5 5H9l2 2h8.5A1.5 1.5 0 0 1 21 8.5v9A1.5 1.5 0 0 1 19.5 19h-15A1.5 1.5 0 0 1 3 17.5z" />
  ),
  save: (
    <>
      <path d="M5 4h11l3 3v13H5z" />
      <path d="M8 4v5h7V4" />
      <path d="M8 13.5h8V20H8z" />
    </>
  ),
  undo: (
    <>
      <polyline points="8 5 3.5 9 8 13" />
      <path d="M3.5 9H14a5 5 0 0 1 0 10H9" />
    </>
  ),
  redo: (
    <>
      <polyline points="16 5 20.5 9 16 13" />
      <path d="M20.5 9H10a5 5 0 0 0 0 10h5" />
    </>
  ),
  "rotate-cw": (
    <>
      <path d="M20 12a8 8 0 1 1-3.2-6.4" />
      <polyline points="20 4 20 8 16 8" />
    </>
  ),
  "rotate-ccw": (
    <>
      <path d="M4 12a8 8 0 1 0 3.2-6.4" />
      <polyline points="4 4 4 8 8 8" />
    </>
  ),
  trash: (
    <>
      <path d="M4 7h16" />
      <path d="M9.5 7V4.5h5V7" />
      <path d="M6.5 7l1 13h9l1-13" />
      <path d="M10.5 11v5.5M13.5 11v5.5" />
    </>
  ),
  "insert-blank": (
    <>
      <path d="M6 3.5h8l4.5 4.5V20.5H6z" />
      <path d="M14 3.5V8h4.5" />
      <path d="M12 11.5v6M9 14.5h6" />
    </>
  ),
  copy: (
    <>
      <rect x="8.5" y="3" width="11.5" height="13" rx="1.5" />
      <rect x="4" y="8" width="11.5" height="13" rx="1.5" />
    </>
  ),
  text: <path d="M8 5h8M12 5v14M8 19h8" />,
  "zoom-in": (
    <>
      <circle cx="11" cy="11" r="6.5" />
      <path d="M20 20l-4.3-4.3M11 8.5v5M8.5 11h5" />
    </>
  ),
  "zoom-out": (
    <>
      <circle cx="11" cy="11" r="6.5" />
      <path d="M20 20l-4.3-4.3M8.5 11h5" />
    </>
  ),
  "fit-page": (
    <>
      <rect x="5" y="3" width="14" height="18" rx="1.5" />
      <rect x="8.5" y="6.5" width="7" height="11" rx="1" />
    </>
  ),
  "fit-width": (
    <>
      <rect x="3" y="5" width="18" height="14" rx="1.5" />
      <path d="M8 12h8M8 12l2.5-2.5M8 12l2.5 2.5M16 12l-2.5-2.5M16 12l-2.5 2.5" />
    </>
  ),
  actual: (
    <>
      <rect x="3" y="6" width="18" height="12" rx="1.5" />
      <text
        x="12"
        y="15.5"
        textAnchor="middle"
        fontSize="8"
        fontWeight="700"
        fill="currentColor"
        stroke="none">
        1:1
      </text>
    </>
  ),
  prev: <polyline points="15 5 8 12 15 19" />,
  next: <polyline points="9 5 16 12 9 19" />,
  bookmark: <path d="M7 3.5h10V20.5l-5-3.8-5 3.8z" />,
  "bookmark-add": (
    <>
      <path d="M6 3.5h9V20.5l-4.5-3.5-4.5 3.5z" />
      <path d="M18 5.5v6M15 8.5h6" />
    </>
  ),
  merge: (
    <>
      <rect x="3" y="4" width="12" height="12" rx="1" />
      <rect x="9" y="8" width="12" height="12" rx="1" />
    </>
  ),
  info: (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 11v5.5" />
      <circle cx="12" cy="7.8" r="0.9" fill="currentColor" stroke="none" />
    </>
  ),
  single: <rect x="7" y="3" width="10" height="18" rx="1.5" />,
  spread: (
    <>
      <rect x="3" y="5" width="8" height="14" rx="1" />
      <rect x="13" y="5" width="8" height="14" rx="1" />
    </>
  ),
  continuous: (
    <>
      <rect x="8" y="2" width="8" height="6" rx="1" />
      <rect x="8" y="9" width="8" height="6" rx="1" />
      <rect x="8" y="16" width="8" height="6" rx="1" />
    </>
  ),
  sidebar: (
    <>
      <rect x="3" y="4" width="18" height="16" rx="1.5" />
      <path d="M9 4v16" />
    </>
  ),
  thumbnails: (
    <>
      <rect x="3" y="3" width="7" height="8" rx="1" />
      <rect x="14" y="3" width="7" height="8" rx="1" />
      <rect x="3" y="13" width="7" height="8" rx="1" />
      <rect x="14" y="13" width="7" height="8" rx="1" />
    </>
  ),
  palette: (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <circle cx="9" cy="10" r="1.1" fill="currentColor" stroke="none" />
      <circle cx="15" cy="10" r="1.1" fill="currentColor" stroke="none" />
      <circle cx="9.5" cy="15" r="1.1" fill="currentColor" stroke="none" />
    </>
  ),
  exit: (
    <>
      <path d="M14 4H6a1.5 1.5 0 0 0-1.5 1.5v13A1.5 1.5 0 0 0 6 20h8" />
      <path d="M17 8l4 4-4 4" />
      <path d="M21 12H10" />
    </>
  ),
};

export function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false">
      {ICONS[name]}
    </svg>
  );
}
