import { save } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useAppState, store } from "../../store";

export function CommitPanel() {
  const app = useAppState();
  const [target, setTarget] = useState<string | null>(null);
  const [overwrite, setOverwrite] = useState(false);
  const [info, setInfo] = useState<{ sha: string; size: number; path: string } | null>(null);
  const [err, setErr] = useState<string | null>(null);

  async function chooseTarget() {
    const picked = await save({
      defaultPath: app.doc
        ? `stitchup-export-from-${app.doc.sourceHash.slice(0, 8)}.pdf`
        : undefined,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (typeof picked === "string") {
      setTarget(picked);
      setErr(null);
    }
  }

  async function doCommit() {
    if (!target) {
      setErr("choose a target path first");
      return;
    }
    setErr(null);
    try {
      const res = await store.commit(target, overwrite);
      setInfo({ sha: res.new_sha256, size: res.size_bytes, path: res.published_path });
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className="commit-panel" aria-label="save / export">
      <span>{target ?? "No target chosen"}</span>
      <button onClick={chooseTarget} disabled={!app.doc}>
        Choose target…
      </button>
      <label className="inline-check">
        <input
          type="checkbox"
          checked={overwrite}
          onChange={(e) => setOverwrite(e.target.checked)}
        />
        overwrite existing
      </label>
      <button onClick={doCommit} disabled={!app.doc || !target}>
        Export (atomic save-as)
      </button>
      {info && (
        <span className="ok-note" title={info.sha}>
          saved {info.size} B → {info.path}
        </span>
      )}
      {err && <span className="err-note">{err}</span>}
    </div>
  );
}