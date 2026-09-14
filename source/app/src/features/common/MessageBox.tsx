import { useAppState, store } from "../../store";

export function MessageBox() {
  const app = useAppState();
  if (!app.lastError) {
    return null;
  }
  return (
    <div className="error-toast" role="alert">
      <span>{app.lastError.message}</span>
      <button onClick={() => store.clearError()}>Dismiss</button>
    </div>
  );
}