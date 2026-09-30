import { useEffect, useRef, useState } from "react";
import { useAppState, store } from "../../store";

/** Password prompt for a protected document (A-c6). */
export function PasswordDialog() {
  const app = useAppState();
  const [password, setPassword] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  const open = app.unlockOpen && !!app.doc?.passwordRequired;

  useEffect(() => {
    if (open) {
      setPassword("");
      inputRef.current?.focus();
    }
  }, [open]);

  if (!open) return null;

  function submit(e: React.FormEvent) {
    e.preventDefault();
    if (password) void store.unlock(password);
  }

  return (
    <div className="modal-backdrop">
      <form className="modal" role="dialog" aria-modal="true" aria-label="unlock document" onSubmit={submit}>
        <h2>Password required</h2>
        <p className="muted">
          This PDF is encrypted. Enter its password to load the page tree.
        </p>
        <label>
          Password
          <input
            ref={inputRef}
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="off"
          />
        </label>
        <div className="modal-actions">
          <button type="submit" disabled={app.busy || password.length === 0}>
            Unlock
          </button>
          <button type="button" onClick={() => store.setUnlockOpen(false)}>
            Cancel
          </button>
        </div>
      </form>
    </div>
  );
}
