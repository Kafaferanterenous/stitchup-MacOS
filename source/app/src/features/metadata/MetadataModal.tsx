import { useState } from "react";
import { useAppState, store } from "../../store";
import type { Metadata } from "../../types";

export function MetadataModal({ open }: { open: boolean }) {
  const app = useAppState();
  const [form, setForm] = useState<Metadata>({});
  const [saved, setSaved] = useState<string | null>(null);

  function commit() {
    if (!app.doc) return;
    void store.updateMetadata({
      title: form.title || undefined,
      author: form.author || undefined,
      subject: form.subject || undefined,
      keywords: form.keywords || undefined,
    });
    setSaved("Saved");
    setTimeout(() => setSaved(null), 1500);
  }

  return open ? (
    <div className="modal-backdrop" onMouseDown={() => store.setMetadataOpen(false)}>
      <div className="modal" role="dialog" aria-modal="true" onMouseDown={(e) => e.stopPropagation()}>
        <h2>Document metadata</h2>
        {saved && <p className="ok-note">{saved}</p>}
        <label>
          Title
          <input
            defaultValue={app.doc?.metadata?.title ?? ""}
            onChange={(e) => setForm((f) => ({ ...f, title: e.target.value }))}
          />
        </label>
        <label>
          Author
          <input
            defaultValue={app.doc?.metadata?.author ?? ""}
            onChange={(e) => setForm((f) => ({ ...f, author: e.target.value }))}
          />
        </label>
        <label>
          Subject
          <input
            defaultValue={app.doc?.metadata?.subject ?? ""}
            onChange={(e) => setForm((f) => ({ ...f, subject: e.target.value }))}
          />
        </label>
        <label>
          Keywords
          <input
            defaultValue={app.doc?.metadata?.keywords ?? ""}
            onChange={(e) => setForm((f) => ({ ...f, keywords: e.target.value }))}
          />
        </label>
        <div className="modal-actions">
          <button onClick={commit}>Save</button>
          <button onClick={() => store.setMetadataOpen(false)}>Close</button>
        </div>
      </div>
    </div>
  ) : null;
}