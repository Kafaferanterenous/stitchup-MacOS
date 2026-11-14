import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useAppState, store, docName, getState } from "./store";
import { COMMANDS, matchesKey, runCommand } from "./commands";
import { installMenu, nativeMenuActive } from "./menu";
import { applyTheme, type ThemeName } from "./themes/theme";
import { cmdClearRecovery, cmdListRecovery, cmdSaveRecovery, cmdTakePendingOpen, type RecoveryEntry } from "./ipc";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { loadSettings, type Settings } from "./settings";
import { Ribbon } from "./features/ribbon/Ribbon";
import { TabStrip } from "./features/common/TabStrip";
import { Viewer } from "./features/viewer/Viewer";
import { PageTree } from "./features/pageops/PageTree";
import { MessageBox } from "./features/common/MessageBox";
import { PasswordDialog } from "./features/common/PasswordDialog";
import {
  AboutDialog,
  AddPageDialog,
  AnnotInputDialog,
  RecoveryDialog,
  SettingsDialog,
  CompressDialog,
  FontDialog,
  RecolorDialog,
  SignDialog,
  AttachmentsDialog,
  EncryptDialog,
  ExportImagesDialog,
  ExtractDialog,
  HelpDialog,
  NewDialog,
  PrintDialog,
  PropertiesDialog,
  SplitDialog,
  WatermarkDialog,
} from "./features/common/Dialogs";

/** True while the user is typing in a field (shortcuts must not steal keys). */
function typing(e: KeyboardEvent): boolean {
  const t = e.target as HTMLElement | null;
  return !!t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable);
}

// Test / automation hook (also handy from the devtools console).
(window as unknown as { __stitchup: unknown }).__stitchup = {
  run: runCommand,
  theme: (t: ThemeName) => applyTheme(t),
  state: getState,
  store,
};

export default function App() {
  const app = useAppState();
  const [dropping, setDropping] = useState(false);
  const [recovered, setRecovered] = useState<RecoveryEntry[] | null>(null);

  // Crash recovery: offer snapshots left by a Stitchup that crashed…
  useEffect(() => {
    cmdListRecovery().then((list) => list.length && setRecovered(list), () => {});
  }, []);
  // …and take snapshots of unsaved tabs at the chosen interval.
  useEffect(() => {
    let timer: ReturnType<typeof setInterval> | null = null;
    const arm = (s: Settings) => {
      if (timer) clearInterval(timer);
      timer = null;
      if (s.recoverySeconds > 0) {
        timer = setInterval(() => {
          if (store.anyDirty()) void cmdSaveRecovery().catch(() => {});
          else void cmdClearRecovery().catch(() => {});
        }, s.recoverySeconds * 1000);
      }
    };
    arm(loadSettings());
    const onChange = (e: Event) => arm((e as CustomEvent<Settings>).detail);
    window.addEventListener("stitchup-settings", onChange);
    return () => {
      if (timer) clearInterval(timer);
      window.removeEventListener("stitchup-settings", onChange);
    };
  }, []);

  // Closing the window with unsaved changes asks first (like ⌘Q).
  useEffect(() => {
    let un: (() => void) | null = null;
    try {
      getCurrentWindow()
        .onCloseRequested(async (e) => {
          if (!(await store.confirmDiscard("Close Stitchup"))) {
            e.preventDefault();
            return;
          }
          await cmdClearRecovery().catch(() => {});
        })
        .then((f) => (un = f), () => {});
    } catch {
      /* not inside Tauri */
    }
    return () => un?.();
  }, []);

  // Native menu + files handed to us by Finder / the Dock / the command line.
  useEffect(() => {
    void installMenu();
    const openFirst = (paths: string[]) => {
      const pdf = paths.find((p) => p.toLowerCase().endsWith(".pdf"));
      if (pdf) void store.open(pdf);
    };
    cmdTakePendingOpen().then(openFirst, () => {});
    const unlisten: Promise<() => void>[] = [];
    // Outside Tauri (browser, tests) these reject: swallow right away.
    const quiet = (p: Promise<() => void>) => p.catch(() => () => {});
    try {
      unlisten.push(quiet(listen<string[]>("open-files", (e) => openFirst(e.payload))));
      unlisten.push(
        quiet(
          getCurrentWebview().onDragDropEvent((e) => {
            const p = e.payload;
            if (p.type === "enter" || p.type === "over") setDropping(true);
            else setDropping(false);
            if (p.type === "drop") openFirst(p.paths);
          }),
        ),
      );
    } catch {
      /* not inside Tauri */
    }
    return () => unlisten.forEach((u) => void u.then((f) => f()));
  }, []);

  // Keyboard: every command's accelerator (when no native menu handles it)
  // and its extra keys; Esc leaves Edit Text mode.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      const inField = typing(e);
      for (const c of COMMANDS) {
        const specs = [...(nativeMenuActive() ? [] : c.accel ? [c.accel] : []), ...(c.keys ?? [])];
        if (!specs.some((k) => matchesKey(e, k))) continue;
        const plain = !e.metaKey && !e.ctrlKey && !e.altKey && !/^F\d+$/.test(e.key);
        // In a text field, plain keys (⌫, PgUp…) and ⌘A/⌘Z belong to the field.
        if (inField && (plain || c.textFallback)) return;
        e.preventDefault();
        void runCommand(c.id);
        return;
      }
      if (e.key === "Escape" && !inField) {
        const st = getState();
        if (st.picked) {
          e.preventDefault();
          store.setPicked(null);
        } else if (st.tool) {
          e.preventDefault();
          store.setTool(null);
        } else if (st.editMode) {
          e.preventDefault();
          store.setEditMode(false);
        }
      }
      if (!inField && (e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "c" && getState().selectionText) {
        e.preventDefault();
        void store.copySelection();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Window title like the Windows build: "name * — Stitchup".
  const dirty = store.isDirty();
  useEffect(() => {
    document.title = app.doc ? `${docName(app.doc)}${dirty ? " *" : ""} — Stitchup` : "Stitchup PDF Editor";
  }, [app.doc, dirty]);

  const warnings = app.doc?.warnings.filter((w) => !/password-protected/.test(w)) ?? [];
  const sel = app.selection.length;

  return (
    <div className="app-shell" data-view={app.viewMode}>
      <Ribbon />
      <TabStrip />
      <main className="app-body" data-pane={app.paneOpen && app.doc ? "open" : "hidden"}>
        <section className="viewer-pane" aria-label="document viewer">
          <Viewer />
        </section>
        {app.paneOpen && app.doc && (
          <aside className="pages-pane" aria-label="page thumbnails">
            <PageTree />
          </aside>
        )}
        {dropping && <div className="drop-overlay">Drop a PDF to open it</div>}
      </main>
      <footer className="status-bar" role="status">
        <span className="status-name">{app.doc ? `${docName(app.doc)}${dirty ? " *" : ""}` : "Stitchup PDF Editor"}</span>
        <span className="status-msg">{app.statusMessage}</span>
        <span className="status-right">
          {warnings.length > 0 && (
            <button className="status-warn" title={warnings.join("\n")} onClick={() => store.setMetadataOpen(true)}>
              ⚠ {warnings.length} warning{warnings.length > 1 ? "s" : ""}
            </button>
          )}
          {app.doc && app.doc.passwordRequired && (
            <button className="status-warn" onClick={() => store.setUnlockOpen(true)}>
              🔒 password required — unlock
            </button>
          )}
          {app.doc && app.doc.encrypted && !app.doc.passwordRequired && <span>unlocked</span>}
          {sel > 1 && <span>{sel} pages selected</span>}
          <span>{app.doc && !app.doc.passwordRequired ? `Page ${app.currentPage} of ${app.doc.pages.length}` : "No document"}</span>
          <span>Zoom {Math.round(app.zoom * 100)}%</span>
        </span>
      </footer>
      <PropertiesDialog />
      {app.dialog === "about" && <AboutDialog />}
      {app.dialog === "extract" && <ExtractDialog />}
      {app.dialog === "split" && <SplitDialog />}
      {app.dialog === "new" && <NewDialog />}
      {app.dialog === "addPage" && <AddPageDialog />}
      {app.dialog === "watermark" && <WatermarkDialog />}
      {app.dialog === "encrypt" && <EncryptDialog />}
      {app.dialog === "attachments" && <AttachmentsDialog />}
      {app.dialog === "exportImages" && <ExportImagesDialog />}
      {app.dialog === "print" && app.doc && <PrintDialog />}
      {app.dialog === "help" && <HelpDialog />}
      {app.dialog === "sign" && <SignDialog />}
      {app.dialog === "recolor" && <RecolorDialog />}
      {app.dialog === "font" && <FontDialog />}
      {app.pendingInput && <AnnotInputDialog />}
      {app.dialog === "settings" && <SettingsDialog />}
      {app.dialog === "compress" && <CompressDialog />}
      {recovered && <RecoveryDialog entries={recovered} onDone={() => setRecovered(null)} />}
      <PasswordDialog />
      <MessageBox />
      {app.notice && (
        <div className="notice-toast" role="status">
          {app.notice}
        </div>
      )}
    </div>
  );
}
