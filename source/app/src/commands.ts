//! The command registry: every ribbon button, menu item and keyboard shortcut
//! is defined once here (labels, tooltips and tab placement follow the
//! Windows build so both look and behave the same).

import { getState, store, isUntitled, docName, type AppState, type RibbonTabName, type Tool } from "./store";
import { extractTo, pickAndMerge, pickAndOpen, saveAs } from "./actions";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { toggleDark } from "./themes/theme";
import type { IconName } from "./features/ribbon/icons";
import * as exp from "./export";
import { cmdReadImage, cmdSaveCompressed, cmdWriteFile } from "./ipc";

export interface Command {
  id: string;
  /** Ribbon caption. */
  label: string;
  /** Menu text when it differs from the ribbon caption. */
  menu?: string;
  tip: string;
  icon: IconName;
  /** Tauri accelerator (native menu) — also matched by the JS key handler. */
  accel?: string;
  /** Extra keys handled in JS only (not shown in menus), e.g. "Delete". */
  keys?: string[];
  /** Ribbon tab the button lives on. */
  tab?: RibbonTabName;
  run: () => unknown;
  enabled?: (s: AppState) => boolean;
  active?: (s: AppState) => boolean;
  /** Not built yet: the GOALS.md item that will deliver it. */
  soon?: string;
  /** Inside a text field, run this editing command instead (undo, select all…). */
  textFallback?: string;
  /** Drop-down choices shown under the ribbon button (Shape, Stamp…). */
  choices?: () => { label: string; run: () => void; active?: boolean }[];
  /** Colour swatch button (Fore / Back). */
  swatch?: "fore" | "back" | "ann";
}

export const STAMPS = ["Approved", "Not Approved", "Draft", "Final", "Confidential", "Paid", "Sold", "Received", "Rejected", "Void", "Urgent", "For Comment"];

const toolCmd = (tool: Tool) => () => store.setTool(tool);
const isTool = (tool: Tool) => (s: AppState) => s.tool === tool;
const shapeChoices = (prefix: "" | "shape-") => () => {
  const s = getState();
  const items: [string, Tool][] = [
    ["Rectangle", (prefix ? "shape-rect" : "rect") as Tool],
    ["Oval", (prefix ? "shape-oval" : "oval") as Tool],
    ["Line", (prefix ? "shape-line" : "line") as Tool],
    ["Arrow", (prefix ? "shape-arrow" : "arrow") as Tool],
  ];
  return items.map(([label, t]) => ({ label, active: s.tool === t, run: () => store.setTool(t) }));
};
const stampChoices = () => {
  const s = getState();
  return [
    ...STAMPS.map((label) => ({
      label,
      active: s.tool === "stamp" && s.stampLabel === label,
      run: () => {
        store.setStampLabel(label);
        if (getState().tool !== "stamp") store.setTool("stamp");
        store.notify(`Stamp “${label}”: click or drag on the page`);
      },
    })),
    {
      label: "Custom…",
      run: () => {
        const t = window.prompt("Stamp text", getState().stampLabel);
        if (t && t.trim()) {
          store.setStampLabel(t.trim());
          if (getState().tool !== "stamp") store.setTool("stamp");
        }
      },
    },
  ];
};

const hasDoc = (s: AppState) => !!s.doc && !s.doc.passwordRequired;
const PDF = [{ name: "PDF", extensions: ["pdf"] }];

function stem(): string {
  const d = getState().doc;
  return docName(d).replace(/\.pdf$/i, "") || "document";
}

function dirOf(): string {
  const d = getState().doc;
  if (!d || isUntitled(d)) return "";
  const i = d.path.lastIndexOf("/");
  return i > 0 ? d.path.slice(0, i + 1) : "";
}

/** Ask for a target file, then write `bytes` there. */
async function exportBytes(ext: string, label: string, make: () => Promise<Uint8Array>) {
  const s = getState();
  if (!s.session) return;
  const target = await saveDialog({ defaultPath: `${dirOf()}${stem()}.${ext}`, filters: [{ name: label, extensions: [ext] }] });
  if (typeof target !== "string") return;
  try {
    const bytes = await make();
    const res = await cmdWriteFile(target, exp.toBase64(bytes));
    store.notify(`Exported ${label} → ${res.published_path}`);
  } catch (e) {
    store.notify(`Export failed: ${(e as Error).message ?? e}`);
  }
}

const lines = () => exp.pageLines(getState().session!);

async function compress() {
  const target = await saveDialog({ defaultPath: `${dirOf()}${stem()}-compressed.pdf`, filters: PDF });
  if (typeof target !== "string") return;
  try {
    const r = await cmdSaveCompressed(target);
    const pct = r.before > 0 ? Math.round((1 - r.after / r.before) * 100) : 0;
    store.notify(`Compressed: ${(r.before / 1024).toFixed(0)} KB → ${(r.after / 1024).toFixed(0)} KB (${pct}% smaller)`);
  } catch (e) {
    store.notify(`Compress failed: ${(e as Error).message ?? e}`);
  }
}

async function mergeMany() {
  const chosen = await openDialog({ multiple: true, filters: PDF });
  const list = Array.isArray(chosen) ? chosen : typeof chosen === "string" ? [chosen] : [];
  for (const p of list) if (!(await store.merge(p))) break;
}

async function combineNew() {
  const chosen = await openDialog({ multiple: true, filters: PDF });
  const list = Array.isArray(chosen) ? chosen : typeof chosen === "string" ? [chosen] : [];
  if (list.length) await store.combine(list);
}

async function save() {
  const s = getState();
  if (isUntitled(s.doc)) await saveAs();
  else await store.save();
}

async function autoCrop() {
  const s = getState();
  if (!s.session) return;
  const boxes: number[][] = [];
  for (const p of store.targets()) {
    const b = await exp.inkBox(s.session, p);
    if (b) boxes.push([p, ...b]);
  }
  if (boxes.length) await store.crop(boxes);
  else store.notify("Nothing to crop: the page is blank");
}

/** Pick an image, decode it (JPEG kept as-is), then arm the Image tool. */
async function insertImage() {
  const p = await openDialog({ multiple: false, filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "heic", "tif", "tiff"] }] });
  if (typeof p !== "string") return;
  try {
    const { b64 } = await cmdReadImage(p);
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    const name = p.split("/").pop() ?? "image";
    const bmp = await createImageBitmap(new Blob([bytes]));
    const isJpeg = bytes[0] === 0xff && bytes[1] === 0xd8;
    if (isJpeg) {
      store.armImage({ format: "jpeg", b64, width: bmp.width, height: bmp.height, name });
      return;
    }
    const c = document.createElement("canvas");
    c.width = bmp.width;
    c.height = bmp.height;
    const ctx = c.getContext("2d")!;
    ctx.drawImage(bmp, 0, 0);
    const px = ctx.getImageData(0, 0, c.width, c.height).data;
    store.armImage({ format: "rgba", b64: exp.toBase64(new Uint8Array(px.buffer)), width: c.width, height: c.height, name });
  } catch (e) {
    store.notify(`Could not read that image: ${(e as Error).message ?? e}`);
  }
}

const zoomStep = (dir: 1 | -1) => {
  const z = getState().zoom;
  store.setZoom(Math.min(5, Math.max(0.25, Math.round((dir > 0 ? z * 1.1 : z / 1.1) * 100) / 100)), "custom");
};

export const COMMANDS: Command[] = [
  // ---------------------------------------------------------------- Document
  { id: "new", label: "New", menu: "New…", tip: "Create a new empty document (⌘N)", icon: "new", accel: "CmdOrCtrl+N", tab: "Document", run: () => store.setDialog("new") },
  { id: "open", label: "Open", menu: "Open…", tip: "Open an existing PDF file (⌘O)", icon: "open", accel: "CmdOrCtrl+O", tab: "Document", run: pickAndOpen },
  { id: "save", label: "Save", tip: "Save changes to the current file (⌘S)", icon: "save", accel: "CmdOrCtrl+S", tab: "Document", run: save, enabled: hasDoc },
  { id: "saveAs", label: "Save As", menu: "Save As…", tip: "Save to a new file name (⌘⇧S)", icon: "save-as", accel: "CmdOrCtrl+Shift+S", tab: "Document", run: saveAs, enabled: hasDoc },
  { id: "properties", label: "Properties", menu: "Document Properties…", tip: "Edit the document metadata (title, author, subject, keywords)", icon: "info", tab: "Document", run: () => store.setMetadataOpen(true), enabled: hasDoc },
  { id: "import", label: "Import", menu: "Import PDF…", tip: "Insert pages from another PDF into this one", icon: "import", tab: "Document", run: async () => {
      const p = await openDialog({ multiple: false, filters: PDF });
      if (typeof p === "string") await store.importPdf(p);
    }, enabled: hasDoc },
  { id: "exportText", label: "Export Text", menu: "Text…", tip: "Write the document text out to a .txt file", icon: "export-text", tab: "Document", enabled: hasDoc, run: () => exportBytes("txt", "Text", async () => exp.toText(await lines())) },
  { id: "exportCsv", label: "Export CSV", menu: "CSV…", tip: "Write page data out to a .csv file", icon: "csv", tab: "Document", enabled: hasDoc, run: () => exportBytes("csv", "CSV", async () => exp.toCsv(getState().session!, await lines())) },
  { id: "exportImages", label: "Images", menu: "Images (JPG / PNG)…", tip: "Export each page as a JPG or PNG image", icon: "image", tab: "Document", enabled: hasDoc, run: () => store.setDialog("exportImages") },
  { id: "exportRtf", label: "RTF", menu: "RTF…", tip: "Write the document text out to an RTF file", icon: "rtf", tab: "Document", enabled: hasDoc, run: () => exportBytes("rtf", "RTF", async () => exp.toRtf(await lines())) },
  { id: "exportDocx", label: "DOCX", menu: "DOCX…", tip: "Write the document text out to a Word .docx file", icon: "docx", tab: "Document", enabled: hasDoc, run: () => exportBytes("docx", "Word document", async () => exp.toDocx(await lines())) },
  { id: "exportOdt", label: "ODT", menu: "ODT…", tip: "Write the document text out to an OpenDocument .odt file", icon: "odt", tab: "Document", enabled: hasDoc, run: () => exportBytes("odt", "OpenDocument text", async () => exp.toOdt(await lines())) },
  { id: "print", label: "Print", menu: "Print…", tip: "Print the document, all pages or a page range (⌘P)", icon: "print", accel: "CmdOrCtrl+P", tab: "Document", enabled: hasDoc, run: () => store.setDialog("print") },
  { id: "printPreview", label: "Preview", menu: "Print Preview", tip: "See how the document looks on paper before printing (⌘⇧P)", icon: "preview", accel: "CmdOrCtrl+Shift+P", tab: "Document", enabled: hasDoc, run: () => store.setDialog("print") },
  { id: "printPdf", label: "to PDF", menu: "Print to PDF…", tip: "Print to a PDF file (choose PDF ▸ Save as PDF in the print panel)", icon: "to-pdf", tab: "Document", enabled: hasDoc, run: () => store.setDialog("print") },
  { id: "mergePdfs", label: "Merge", menu: "Merge PDFs…", tip: "Append one or more PDFs to this document", icon: "merge", enabled: hasDoc, run: mergeMany },
  { id: "combineNew", label: "Combine", menu: "Combine into New…", tip: "Combine several PDFs into a new document", icon: "merge", run: combineNew },
  { id: "compress", label: "Compress", menu: "Compress PDF…", tip: "Save a smaller copy (unused data removed, streams compressed)", icon: "compress", enabled: hasDoc, run: compress },
  { id: "settings", label: "Settings", menu: "Settings…", tip: "Crash recovery, opening zoom, sidebar (⌘,)", icon: "info", accel: "CmdOrCtrl+,", run: () => store.setDialog("settings") },
  { id: "newTab", label: "New Tab", tip: "Open an empty tab (⌘T)", icon: "new", accel: "CmdOrCtrl+T", run: () => store.newTab() },
  { id: "closeTab", label: "Close Tab", tip: "Close this tab (⌘W)", icon: "exit", accel: "CmdOrCtrl+W", run: () => store.closeTab(getState().activeTab) },
  { id: "nextTab", label: "Next Tab", tip: "Next tab (⌃Tab)", icon: "next", accel: "Ctrl+Tab", enabled: (s) => s.tabs.length > 1, run: () => store.cycleTab(1) },
  { id: "prevTab", label: "Previous Tab", tip: "Previous tab (⌃⇧Tab)", icon: "prev", accel: "Ctrl+Shift+Tab", enabled: (s) => s.tabs.length > 1, run: () => store.cycleTab(-1) },
  { id: "exit", label: "Exit", menu: "Quit Stitchup", tip: "Quit (⌘Q)", icon: "exit", accel: "CmdOrCtrl+Q", run: () => store.exitApp() },

  // ------------------------------------------------------------------- Pages
  { id: "rotateCcw", label: "Rotate CCW", menu: "Rotate Left", tip: "Rotate the current page(s) 90° counter-clockwise (⌘⇧R)", icon: "rotate-ccw", accel: "CmdOrCtrl+Shift+R", tab: "Pages", enabled: hasDoc, run: () => store.rotate(store.targets(), false) },
  { id: "rotateCw", label: "Rotate CW", menu: "Rotate Right", tip: "Rotate the current page(s) 90° clockwise (⌘R)", icon: "rotate-cw", accel: "CmdOrCtrl+R", tab: "Pages", enabled: hasDoc, run: () => store.rotate(store.targets(), true) },
  { id: "rotateAll", label: "Rotate All", menu: "Rotate All Pages Right", tip: "Rotate every page 90° clockwise", icon: "rotate-cw", enabled: hasDoc, run: () => store.rotate(getState().doc!.pages.map((p) => p.page_no), true) },
  { id: "delete", label: "Delete", menu: "Delete Page(s)", tip: "Delete the current page(s) (⌫)", icon: "trash", keys: ["Delete", "Backspace"], tab: "Pages",
    enabled: (s) => hasDoc(s) && (!!s.picked || (s.selection.length || 1) < (s.doc?.pages.length ?? 0)),
    // ⌫ with objects selected (Select tool) deletes those, not pages.
    run: () => (getState().picked ? store.deletePicked() : store.deletePages(store.targets())) },
  { id: "addPage", label: "Add Page", menu: "Add Page…", tip: "Add a blank page after the current page", icon: "insert-blank", tab: "Pages", enabled: hasDoc, run: () => store.setDialog("addPage") },
  { id: "extract", label: "Extract", menu: "Extract Selected Page(s)…", tip: "Save the selected page(s) as their own PDF file", icon: "extract", tab: "Pages", enabled: hasDoc, run: () => (getState().selection.length ? extractTo(store.targets()) : store.setDialog("extract")) },
  { id: "split", label: "Split", menu: "Split Document…", tip: "Split the document into several files", icon: "split", tab: "Pages", enabled: (s) => hasDoc(s) && (s.doc?.pages.length ?? 0) > 1, run: () => store.setDialog("split") },
  { id: "autoCrop", label: "Auto-Crop", menu: "Auto-Crop Selected Page(s)", tip: "Trim the white margin around the selected page(s)", icon: "crop", tab: "Pages", enabled: hasDoc, run: autoCrop },
  { id: "selectAll", label: "Select All", menu: "Select All Pages", tip: "Select every page (⌘A)", icon: "thumbnails", accel: "CmdOrCtrl+A", enabled: hasDoc, run: () => store.selectAllPages(), textFallback: "selectAll" },
  { id: "merge", label: "Merge PDF", tip: "Append the pages of another PDF", icon: "merge", enabled: hasDoc, run: pickAndMerge },

  // ---------------------------------------------------------------- Annotate
  { id: "annHighlight", label: "Highlight", tip: "Drag across text to highlight it", icon: "highlight", tab: "Annotate", enabled: hasDoc, active: isTool("highlight"), run: toolCmd("highlight") },
  { id: "annUnderline", label: "Underline", tip: "Drag across text to underline it (Fore colour)", icon: "underline", tab: "Annotate", enabled: hasDoc, active: isTool("underline"), run: toolCmd("underline") },
  { id: "annStrike", label: "Strike", menu: "Strike-out", tip: "Drag across text to strike it out (Fore colour)", icon: "strike", tab: "Annotate", enabled: hasDoc, active: isTool("strikeout"), run: toolCmd("strikeout") },
  { id: "annNote", label: "Note", menu: "Sticky Note", tip: "Click to attach a sticky note comment", icon: "note", tab: "Annotate", enabled: hasDoc, active: isTool("note"), run: toolCmd("note") },
  { id: "annText", label: "Text Box", tip: "Drag a box, then type its text", icon: "textbox", tab: "Annotate", enabled: hasDoc, active: isTool("textbox"), run: toolCmd("textbox") },
  { id: "annShape", label: "Shape", tip: "Draw a rectangle, oval, line or arrow (Fore outline, Back fill)", icon: "shape", tab: "Annotate", enabled: hasDoc,
    active: (s) => ["rect", "oval", "line", "arrow"].includes(s.tool ?? ""), choices: shapeChoices(""), run: () => {} },
  { id: "annStamp", label: "Stamp", tip: "Choose a stamp, then click or drag on the page", icon: "stamp", tab: "Annotate", enabled: hasDoc, active: isTool("stamp"), choices: stampChoices, run: () => {} },
  { id: "annLink", label: "Link", menu: "Link…", tip: "Drag the clickable area, then give a web address or page number", icon: "link", tab: "Annotate", enabled: hasDoc, active: isTool("link"), run: toolCmd("link") },
  { id: "annRedact", label: "Redact", tip: "Drag over content to permanently remove it and black it out", icon: "redact", tab: "Annotate", enabled: hasDoc, active: isTool("redact"), run: toolCmd("redact") },
  { id: "annPen", label: "Pen", tip: "Draw freehand lines on the page (Fore colour)", icon: "pen", tab: "Annotate", enabled: hasDoc, active: isTool("pen"), run: toolCmd("pen") },
  { id: "annFore", label: "Colour", tip: "Colour for new annotations: highlight, underline, strike-out, pen, shapes and text boxes", icon: "palette", tab: "Annotate", swatch: "ann", run: () => {} },

  // ----------------------------------------------------------------- Content
  { id: "select", label: "Select", menu: "Select / Move Content Object", tip: "Click text, images or annotations to select (⌘-click adds, drag a box for many); drag to move; ⌫ deletes", icon: "select", tab: "Content", enabled: hasDoc, active: isTool("select"), run: toolCmd("select") },
  { id: "editText", label: "Edit Text", menu: "Edit Text", tip: "Click any line of text to edit it in place, in its own font (or just double-click a line)", icon: "text", tab: "Content", enabled: hasDoc, active: (s) => s.editMode, run: () => {
      const on = getState().editMode;
      store.setTool(null); // (also leaves Edit Text mode)
      store.setEditMode(!on);
    } },
  { id: "objDelete", label: "Delete", menu: "Delete Selected Object", tip: "Remove the selected objects (⌫)", icon: "trash", tab: "Content",
    enabled: (s) => hasDoc(s) && !!s.picked && s.picked.objects.length + s.picked.annots.length > 0, run: () => store.deletePicked() },
  { id: "recolor", label: "Recolor", menu: "Recolor Selected Object…", tip: "Change the colour of the selected text or the line being edited", icon: "palette", tab: "Content",
    enabled: (s) => hasDoc(s) && (!!s.picked?.objects.length || !!s.editingLine), run: () => store.setDialog("recolor") },
  { id: "font", label: "Font", menu: "Font and Size…", tip: "Change the font family and size of the selected text or the line being edited", icon: "font", tab: "Content",
    enabled: (s) => hasDoc(s) && (!!s.picked?.objects.length || !!s.editingLine), run: () => store.setDialog("font") },
  { id: "fore", label: "Fore", tip: "Text colour: recolours the selected text or the line being edited; also the outline of new shapes", icon: "palette", tab: "Content", swatch: "fore", run: () => {} },
  { id: "back", label: "Back", tip: "Fill colour for new shapes (or none)", icon: "palette", tab: "Content", swatch: "back", run: () => {} },
  { id: "image", label: "Image", menu: "Insert Image…", tip: "Insert a picture (PNG, JPEG, GIF, WebP, HEIC…): choose it, then click or drag on the page", icon: "image", tab: "Content", enabled: hasDoc, active: isTool("image"), run: insertImage },
  { id: "ocr", label: "OCR", menu: "Recognize Text (OCR)", tip: "Make scanned pages searchable: recognise their text on this Mac (Apple Vision) and add it as an invisible layer", icon: "find", tab: "Content", enabled: hasDoc,
    choices: () => [
      { label: "Pages Without Text", run: () => void store.recognizeText("untexted") },
      { label: "This Page", run: () => void store.recognizeText("current") },
      { label: "All Pages", run: () => void store.recognizeText("all") },
    ], run: () => {} },
  { id: "shapes", label: "Shapes", tip: "Insert a vector shape into the page content; click for a default size or drag", icon: "shape", tab: "Content", enabled: hasDoc,
    active: (s) => (s.tool ?? "").startsWith("shape-"), choices: shapeChoices("shape-"), run: () => {} },

  // -------------------------------------------------------------------- Zoom
  { id: "zoomOut", label: "Zoom -", menu: "Zoom Out", tip: "Zoom out one step (⌘−)", icon: "zoom-out", accel: "CmdOrCtrl+-", tab: "Zoom", enabled: hasDoc, run: () => zoomStep(-1) },
  { id: "zoomIn", label: "Zoom +", menu: "Zoom In", tip: "Zoom in one step (⌘=)", icon: "zoom-in", accel: "CmdOrCtrl+=", tab: "Zoom", enabled: hasDoc, run: () => zoomStep(1) },
  { id: "zoom100", label: "100%", tip: "Reset the zoom to 100% (⌘1)", icon: "actual", accel: "CmdOrCtrl+1", tab: "Zoom", enabled: hasDoc, active: (s) => s.zoomMode === "actual", run: () => store.setZoom(1, "actual") },
  { id: "fitWidth", label: "Fit Width", tip: "Scale the page so its width fills the window (⌘2)", icon: "fit-width", accel: "CmdOrCtrl+2", tab: "Zoom", enabled: hasDoc, active: (s) => s.zoomMode === "fit-width", run: () => store.setFitMode("fit-width") },
  { id: "fitPage", label: "Fit Page", tip: "Scale the page so the whole page is visible (⌘0)", icon: "fit-page", accel: "CmdOrCtrl+0", tab: "Zoom", enabled: hasDoc, active: (s) => s.zoomMode === "fit-page", run: () => store.setFitMode("fit-page") },

  // ---------------------------------------------------------------- Navigate
  { id: "prev", label: "Previous", menu: "Previous Page", tip: "Go to the previous page (⌘↑ / PgUp)", icon: "prev", accel: "CmdOrCtrl+Up", keys: ["PageUp"], tab: "Navigate", enabled: (s) => hasDoc(s) && s.currentPage > 1, run: () => store.goToPage(getState().currentPage - 1) },
  { id: "next", label: "Next", menu: "Next Page", tip: "Go to the next page (⌘↓ / PgDn)", icon: "next", accel: "CmdOrCtrl+Down", keys: ["PageDown"], tab: "Navigate", enabled: (s) => hasDoc(s) && s.currentPage < (s.doc?.pages.length ?? 0), run: () => store.goToPage(getState().currentPage + 1) },
  { id: "spread", label: "Spread", menu: "Two-Page Spread", tip: "Show facing pages side by side (F5)", icon: "spread", accel: "F5", tab: "Navigate", enabled: hasDoc, active: (s) => s.viewMode === "spread", run: () => store.setViewMode(getState().viewMode === "spread" ? "continuous" : "spread") },
  { id: "single", label: "Single", menu: "Single Page", tip: "One page at a time", icon: "single", tab: "Navigate", enabled: hasDoc, active: (s) => s.viewMode === "single", run: () => store.setViewMode("single") },
  { id: "continuous", label: "Continuous", menu: "Continuous Scroll", tip: "All pages in one scrolling column", icon: "continuous", tab: "Navigate", enabled: hasDoc, active: (s) => s.viewMode === "continuous", run: () => store.setViewMode("continuous") },
  { id: "sidebar", label: "Sidebar", tip: "Show or hide the page thumbnails (F8)", icon: "sidebar", accel: "F8", tab: "Navigate", active: (s) => s.paneOpen, run: () => store.setPaneOpen(!getState().paneOpen) },

  // ---------------------------------------------------------------- Security
  { id: "watermark", label: "Watermark", menu: "Watermark…", tip: "Stamp a text watermark across every page", icon: "watermark", tab: "Security", enabled: hasDoc, run: () => store.setDialog("watermark") },
  { id: "stampMenu", label: "Stamp…", tip: "Place a common status stamp (Draft, Approved, Paid…)", icon: "stamp", tab: "Security", enabled: hasDoc, active: isTool("stamp"), choices: stampChoices, run: () => {} },
  { id: "encrypt", label: "Encrypt", menu: "Save As Encrypted…", tip: "Save a copy protected by a password (AES-256)", icon: "lock", tab: "Security", enabled: hasDoc, run: () => store.setDialog("encrypt") },
  { id: "sign", label: "Sign", menu: "Sign Document…", tip: "Add a digital signature and save a signed copy", icon: "sign", tab: "Security", enabled: hasDoc, active: isTool("sign"), run: () => store.setDialog("sign") },

  // -------------------------------------------------------------- Edit / misc
  { id: "undo", label: "Undo", tip: "Undo the last change (⌘Z)", icon: "undo", accel: "CmdOrCtrl+Z", enabled: (s) => hasDoc(s) && (s.doc?.history.length ?? 0) > 0, run: () => store.undo(), textFallback: "undo" },
  { id: "redo", label: "Redo", tip: "Redo (⌘⇧Z / ⌘Y)", icon: "redo", accel: "CmdOrCtrl+Shift+Z", keys: ["Meta+y", "Control+y"], enabled: (s) => hasDoc(s) && (s.doc?.redo.length ?? 0) > 0, run: () => store.redo(), textFallback: "redo" },
  { id: "find", label: "Find", menu: "Find…", tip: "Search the document text (⌘F)", icon: "find", accel: "CmdOrCtrl+F", enabled: hasDoc, run: () => {
      const el = document.querySelector<HTMLInputElement>(".search-box input");
      el?.focus();
      el?.select();
    } },
  { id: "findNext", label: "Find Next", tip: "Next match (F3 / ⌘G)", icon: "next", accel: "F3", keys: ["Meta+g"], enabled: hasDoc, run: () => store.findNext(1) },
  { id: "findPrev", label: "Find Previous", tip: "Previous match (⇧F3 / ⌘⇧G)", icon: "prev", accel: "Shift+F3", keys: ["Meta+Shift+g"], enabled: hasDoc, run: () => store.findNext(-1) },
  { id: "copy", label: "Copy", tip: "Copy the selected text (⌘C)", icon: "copy", enabled: (s) => s.selectionText.length > 0, run: () => store.copySelection() },
  { id: "darkMode", label: "Dark Mode", tip: "Flip between Light Grey and Dark Blue (⌘D)", icon: "palette", accel: "CmdOrCtrl+D", run: () => toggleDark() },
  { id: "guide", label: "User Guide", tip: "How to use Stitchup (F1)", icon: "info", accel: "F1", run: () => store.setDialog("help") },
  { id: "about", label: "About", menu: "About Stitchup", tip: "Version and open-source licenses", icon: "info", run: () => store.setDialog("about") },
];

export const byId = new Map(COMMANDS.map((c) => [c.id, c]));

/** Run a command by id, honouring `enabled` and text-field fallbacks. */
export async function runCommand(id: string): Promise<void> {
  const c = byId.get(id);
  if (!c) return;
  const ae = document.activeElement as HTMLElement | null;
  const typing = !!ae && (ae.tagName === "INPUT" || ae.tagName === "TEXTAREA" || ae.isContentEditable);
  if (typing && c.textFallback) {
    document.execCommand(c.textFallback);
    return;
  }
  const s = getState();
  if (c.soon) {
    store.notify(`${c.label} is coming soon (goal ${c.soon} in GOALS.md)`);
    return;
  }
  if (c.enabled && !c.enabled(s)) {
    if (!s.doc) store.notify("Open a document first");
    return;
  }
  if (c.choices) {
    // From the menu bar / shortcuts: open the ribbon tab so its drop-down shows.
    if (c.tab) store.setRibbonTab(c.tab);
    window.dispatchEvent(new CustomEvent("stitchup-choices", { detail: c.id }));
    return;
  }
  if (c.swatch) {
    window.dispatchEvent(new CustomEvent("stitchup-swatch", { detail: c.swatch }));
    return;
  }
  await c.run();
}

/** Does a keyboard event match a Tauri accelerator / key string? */
export function matchesKey(e: KeyboardEvent, spec: string): boolean {
  const parts = spec.split("+");
  const key = parts.pop()!;
  const want = {
    cmd: parts.some((p) => p === "CmdOrCtrl" || p === "Meta" || p === "Control" || p === "Ctrl" || p === "Cmd"),
    shift: parts.includes("Shift"),
    alt: parts.includes("Alt"),
  };
  if ((e.metaKey || e.ctrlKey) !== want.cmd || e.shiftKey !== want.shift || e.altKey !== want.alt) return false;
  const k = e.key;
  if (key === "=") return k === "=" || k === "+";
  if (key === "-") return k === "-" || k === "_";
  if (key === ",") return k === ",";
  if (key === "Up") return k === "ArrowUp";
  if (key === "Down") return k === "ArrowDown";
  return k.toLowerCase() === key.toLowerCase();
}

/** Menu bar layout (same menus as the Windows build). */
export const MENUS: { title: string; items: (string | "-" | { submenu: string })[] }[] = [
  { title: "File", items: ["new", "open", "newTab", "closeTab", "prevTab", "nextTab", "-", "save", "saveAs", "encrypt", "import", "mergePdfs", "combineNew", "compress", "watermark", "sign", "properties", "-", "print", "printPreview", "printPdf", "-", "settings"] },
  { title: "Export", items: ["exportText", "exportCsv", "exportImages", "exportRtf", "exportDocx", "exportOdt"] },
  { title: "Edit", items: ["undo", "redo", "-", { submenu: "clipboard" }, "-", "find", "findNext", "findPrev", "-", "rotateCw", "rotateCcw", "rotateAll", "delete", "addPage", "-", "selectAll", "-", "select", "editText", "objDelete", "recolor", "font", "image", "shapes", "ocr"] },
  { title: "Pages", items: ["extract", "split", "autoCrop"] },
  { title: "Annotate", items: ["annHighlight", "annUnderline", "annStrike", "annNote", "annText", "annShape", "annStamp", "annLink", "annRedact", "annPen"] },
  { title: "View", items: ["zoomIn", "zoomOut", "zoom100", "fitWidth", "fitPage", "-", "prev", "next", "-", "spread", "single", "continuous", "sidebar", "-", { submenu: "schemes" }] },
  { title: "Tabs", items: [{ submenu: "ribbonTabs" }] },
  { title: "Help", items: ["guide", "-", "about"] },
];
