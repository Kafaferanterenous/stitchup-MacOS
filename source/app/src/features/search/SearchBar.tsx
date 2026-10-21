import { useState } from "react";
import { useAppState, store } from "../../store";
import { search } from "../../adapters/renderAdapter";

export function SearchBar() {
  const app = useAppState();
  const [q, setQ] = useState("");
  const [idx, setIdx] = useState(0);
  const hits = app.searchHits;

  async function run() {
    const session = app.session;
    if (!app.doc || !q.trim() || !session) return;
    const all = await search(session, q.trim(), {}, [1, app.doc.pages.length]);
    store.setSearch(all, all.length ? 0 : -1);
    setIdx(0);
    if (all.length) {
      store.goToPage(all[0].page_no);
    }
  }

  function next(step: number) {
    if (!hits.length) return;
    const n = (idx + step + hits.length) % hits.length;
    setIdx(n);
    store.setSearchIndex(n);
    store.goToPage(hits[n].page_no);
  }

  return (
    <div className="search-bar" role="search">
      <input
        value={q}
        onChange={(e) => {
          setQ(e.target.value);
          if (!e.target.value) {
            store.setSearch([], -1);
            setIdx(0);
          }
        }}
        onKeyDown={(e) => e.key === "Enter" && void run()}
        placeholder="Search text (Stage 1, text layer)…"
        disabled={!app.doc}
      />
      <button onClick={() => void run()} disabled={!app.doc}>
        Search
      </button>
      <button onClick={() => next(-1)} disabled={!hits.length}>
        ‹
      </button>
      <button onClick={() => next(1)} disabled={!hits.length}>
        ›
      </button>
      <span className="search-status">
        {hits.length ? `${idx + 1}/${hits.length}` : q ? "no matches" : ""}
      </span>
    </div>
  );
}