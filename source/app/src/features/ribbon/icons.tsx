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
  | "exit"
  | "new"
  | "save-as"
  | "attachment"
  | "import"
  | "export-text"
  | "csv"
  | "image"
  | "rtf"
  | "docx"
  | "odt"
  | "print"
  | "preview"
  | "to-pdf"
  | "compress"
  | "extract"
  | "split"
  | "crop"
  | "highlight"
  | "underline"
  | "note"
  | "textbox"
  | "shape"
  | "stamp"
  | "link"
  | "redact"
  | "pen"
  | "select"
  | "font"
  | "watermark"
  | "lock"
  | "sign"
  | "find"
  | "strike";

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
  "new": (
    <><path d="M6 3h8l4 4v14H6z" /><path d="M14 3v4h4" /></>
  ),
  "save-as": (
    <><path d="M5 4h11l3 3v6" /><path d="M5 4v16h7" /><path d="M8 4v5h7V4" /><path d="M14.5 20.5l1-3.5 5-5 2.5 2.5-5 5z" /></>
  ),
  "attachment": (
    <path d="M16.5 7.5l-7.8 7.8a2.2 2.2 0 0 0 3.1 3.1l8-8a4 4 0 0 0-5.7-5.7l-8.3 8.3a5.8 5.8 0 0 0 8.2 8.2l6-6" />
  ),
  "import": (
    <><path d="M13 3h5l3 3v15H8v-5" /><path d="M3 11h11" /><path d="M10 7.5L13.5 11 10 14.5" /></>
  ),
  "export-text": (
    <><path d="M6 3h8l4 4v14H6z" /><path d="M9 11h6M9 14h6M9 17h4" /></>
  ),
  "csv": (
    <><rect x="3.5" y="4" width="17" height="16" rx="1.5" /><path d="M3.5 9h17M3.5 14h17M9.5 4v16M15 4v16" /></>
  ),
  "image": (
    <><rect x="3.5" y="4.5" width="17" height="15" rx="1.5" /><circle cx="9" cy="10" r="1.6" /><path d="M4 18l5-5 4 4 3-3 4 4" /></>
  ),
  "rtf": (
    <><path d="M6 3h8l4 4v14H6z" /><path d="M9 11h6M9 14h4M9 17h6" /><path d="M14 3v4h4" /></>
  ),
  "docx": (
    <><path d="M6 3h8l4 4v14H6z" /><path d="M8.5 11l1.2 6 1.6-4.5L13 17l1.4-6" /></>
  ),
  "odt": (
    <><path d="M6 3h8l4 4v14H6z" /><circle cx="12" cy="14" r="3" /></>
  ),
  "print": (
    <><path d="M7 9V3.5h10V9" /><rect x="3.5" y="9" width="17" height="8" rx="1.5" /><path d="M7 14h10v6.5H7z" /></>
  ),
  "preview": (
    <><path d="M7 9V3.5h10V9" /><rect x="3.5" y="9" width="17" height="8" rx="1.5" /><circle cx="12" cy="17.5" r="3" /><path d="M14.2 19.7L16 21.5" /></>
  ),
  "to-pdf": (
    <><path d="M6 3h8l4 4v14H6z" /><path d="M9 17v-5h1.7a1.4 1.4 0 0 1 0 2.8H9" /><path d="M13.5 12v5" /></>
  ),
  "compress": (
    <><path d="M6 3h12v18H6z" /><path d="M12 3v2M12 7v2M12 11v2" /><rect x="10.5" y="14" width="3" height="4" rx="0.8" /></>
  ),
  "extract": (
    <><path d="M5 4h9v16H5z" /><path d="M12 12h9" /><path d="M18 8.5l3.5 3.5-3.5 3.5" /></>
  ),
  "split": (
    <><path d="M4 4h7v16H4zM13 4h7v16h-7z" /><path d="M12 2v20" strokeDasharray="2 2" /></>
  ),
  "crop": (
    <><path d="M6 2v14a2 2 0 0 0 2 2h14" /><path d="M2 6h14a2 2 0 0 1 2 2v14" /></>
  ),
  "highlight": (
    <><path d="M5 20h14" /><path d="M8 16l-1.5 2.5h4L12 16" /><path d="M8 16l7-11 4 2.5-6 11z" /></>
  ),
  "underline": (
    <><path d="M7 4v7a5 5 0 0 0 10 0V4" /><path d="M5 20h14" /></>
  ),
  "note": (
    <><path d="M4 4h16v11l-5 5H4z" /><path d="M15 20v-5h5" /><path d="M8 9h8M8 12h5" /></>
  ),
  "textbox": (
    <><rect x="3" y="5" width="18" height="14" rx="1.5" strokeDasharray="3 2" /><path d="M8.5 9h7M12 9v7" /></>
  ),
  "shape": (
    <><rect x="3.5" y="10" width="10" height="10" rx="1" /><circle cx="16" cy="8" r="5" /></>
  ),
  "stamp": (
    <><path d="M9 12V8.5a3 3 0 1 1 6 0V12" /><path d="M4 12h16v4H4z" /><path d="M5 20h14" /></>
  ),
  "link": (
    <><path d="M10 14a4.5 4.5 0 0 0 6.4 0l3-3a4.5 4.5 0 0 0-6.4-6.4l-1 1" /><path d="M14 10a4.5 4.5 0 0 0-6.4 0l-3 3a4.5 4.5 0 0 0 6.4 6.4l1-1" /></>
  ),
  "redact": (
    <><rect x="3.5" y="5" width="17" height="14" rx="1.5" /><rect x="6.5" y="9" width="11" height="3" fill="currentColor" stroke="none" /><path d="M6.5 15.5h6" /></>
  ),
  "pen": (
    <><path d="M14.5 4.5l5 5L9 20H4v-5z" /><path d="M12.5 6.5l5 5" /></>
  ),
  "select": (
    <path d="M5 3l14 7.5-6 1.5-2.5 6.5z" />
  ),
  "font": (
    <><path d="M4 19L9.5 5h1L16 19" /><path d="M6.5 14h7" /><path d="M17 19v-5.5a2.5 2.5 0 0 1 5 0V19M17 16h5" /></>
  ),
  "watermark": (
    <><path d="M6 3h8l4 4v14H6z" /><path d="M8 17l8-8" strokeDasharray="2 1.6" /></>
  ),
  "lock": (
    <><rect x="5" y="10.5" width="14" height="10" rx="1.5" /><path d="M8 10.5V7.5a4 4 0 0 1 8 0v3" /><circle cx="12" cy="15.5" r="1.2" /></>
  ),
  "sign": (
    <><path d="M3 18c3-1 4-5 6-5s1 4 3 4 3-3 4-3 1.5 2 5 2" /><path d="M3 21h18" /></>
  ),
  "find": (
    <><circle cx="10.5" cy="10.5" r="6" /><path d="M15 15l5.5 5.5" /></>
  ),
  "strike": (
    <><path d="M5 12h14" /><path d="M16 6.5A4 4 0 0 0 12 5c-2.5 0-4 1.3-4 3 0 1.6 1.2 2.5 3 3" /><path d="M8 17.5A4.5 4.5 0 0 0 12 19c2.6 0 4.2-1.4 4.2-3.2 0-.9-.3-1.6-.9-2.1" /></>
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
