import { useState } from "react";
import { api, Task } from "../api";
import { Confidence, Empty, PRIORITY_LABEL, SourceNote, TYPE_LABEL, confLevel, dueClass, fmtDue, localInputValue, useAct, useData } from "../components/ui";

export function Review({ open }: { open: (id: string) => void }) {
  const [items, reload] = useData<Task[]>(() => api.tasks({ review_only: true, sort: "created" }));
  const [sel, setSel] = useState<Set<string>>(new Set());
  const act = useAct();
  if (!items) return <div className="page"><p className="muted">Loading…</p></div>;
  const high = items.filter((t) => confLevel(t.confidence) === 3 && !t.possible_duplicate_of);
  const toggle = (id: string) => setSel((s) => { const n = new Set(s); n.has(id) ? n.delete(id) : n.add(id); return n; });
  const bulk = (action: string) => act(() => api.review(action, [...sel]), (n) => `${n} item${n === 1 ? "" : "s"} ${action === "accept" ? "accepted" : "rejected"}`).then(() => { setSel(new Set()); reload(); });

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Review</h1>
          <div className="sub">Commitments Kept found but isn't sure about. Nothing here reminds you until you accept it.</div>
        </div>
        <div className="btn-row">
          {sel.size > 0 ? (
            <>
              <span className="muted small">{sel.size} selected</span>
              <button className="btn primary" onClick={() => bulk("accept")}>Accept selected</button>
              <button className="btn danger" onClick={() => bulk("reject")}>Reject selected</button>
            </>
          ) : (
            high.length > 1 && <button className="btn" onClick={() => setSel(new Set(high.map((h) => h.id)))}>Select {high.length} high-confidence items</button>
          )}
        </div>
      </div>
      <div className="list">
        {items.length === 0 && <Empty title="You're all caught up.">New suggestions appear here after each sync.</Empty>}
        {items.map((t) => <ReviewItem key={t.id} t={t} selected={sel.has(t.id)} toggle={() => toggle(t.id)} open={open} reload={reload} />)}
      </div>
    </div>
  );
}

function ReviewItem({ t, selected, toggle, open, reload }: { t: Task; selected: boolean; toggle: () => void; open: (id: string) => void; reload: () => void }) {
  const act = useAct();
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(t.title);
  const [due, setDue] = useState(localInputValue(t.due_at));
  const run = (p: Promise<unknown>, msg: string) => act(() => p, msg).then(reload);
  return (
    <div className="rv" style={{ background: selected ? "var(--pine-wash)" : undefined }}>
      <input type="checkbox" checked={selected} onChange={toggle} aria-label="Select" style={{ marginTop: 4 }} />
      <div style={{ minWidth: 0 }}>
        {editing ? (
          <div className="grid2" style={{ marginBottom: 8 }}>
            <label className="field">Title<input className="input" value={title} onChange={(e) => setTitle(e.target.value)} /></label>
            <label className="field">Deadline<input className="input" type="datetime-local" value={due} onChange={(e) => setDue(e.target.value)} /></label>
          </div>
        ) : (
          <div className="t-title" style={{ fontSize: 15, cursor: "pointer" }} onClick={() => open(t.id)}>{t.title}</div>
        )}
        <div className="t-meta">
          <span><Confidence value={t.confidence} /> {confLevel(t.confidence) === 3 ? "High confidence" : confLevel(t.confidence) === 2 ? "Medium confidence" : "Low confidence"}</span>
          <span className="pill">{TYPE_LABEL[t.task_type] ?? t.task_type}</span>
          {t.due_at ? <span className={dueClass(t)}>{fmtDue(t.due_at)}{t.due_kind === "inferred" ? " (inferred)" : ""}</span> : <span>No deadline stated</span>}
          {t.priority <= 2 && <span className="pill amber">{PRIORITY_LABEL[t.priority]}</span>}
          <span>{t.origin === "ai" ? "Found by AI" : "Found by rules"}</span>
        </div>
        {t.review_reason && <div className="rv-why">{t.review_reason}</div>}
        <SourceNote s={t.primary_source} />
        <div className="rv-actions">
          {editing ? (
            <>
              <button className="btn sm primary" onClick={async () => {
                await act(() => api.updateTask(t.id, { title, ...(due !== localInputValue(t.due_at) ? { due_at: due ? new Date(due).toISOString() : null } : {}) }));
                run(api.review("accept", [t.id]), "Saved and accepted");
              }}>Save and accept</button>
              <button className="btn sm ghost" onClick={() => setEditing(false)}>Cancel</button>
            </>
          ) : (
            <>
              <button className="btn sm primary" onClick={() => run(api.review("accept", [t.id]), "Accepted")}>Accept</button>
              <button className="btn sm" onClick={() => setEditing(true)}>Edit</button>
              {t.possible_duplicate_of && <button className="btn sm" onClick={() => run(api.review("merge", [t.id], t.possible_duplicate_of!), "Merged into the existing task")}>Merge with existing</button>}
              <button className="btn sm" onClick={() => run(api.review("waiting", [t.id]), "Marked as waiting on someone")}>Waiting on someone</button>
              <select className="input" style={{ width: "auto", padding: "2px 8px", fontSize: 12.5 }} value="" onChange={(e) => e.target.value && run(api.updateTask(t.id, { my_role: e.target.value }), "Ownership changed")} aria-label="Change owner">
                <option value="">Change owner…</option>
                <option value="owner">It's mine</option>
                <option value="collaborator">I'm helping</option>
                <option value="requester">Someone else owns it</option>
              </select>
              <button className="btn sm danger" onClick={() => run(api.review("reject", [t.id]), "Rejected — Kept won't suggest it again")}>Not a task</button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
