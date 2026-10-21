import { useMemo, useState } from "react";
import { Plus } from "lucide-react";
import { api, Task } from "../api";
import { NewTaskModal, TaskRow } from "../components/tasks";
import { Empty, STATUS_LABEL, TYPE_LABEL, dueClass, fmtDue, useAct, useData } from "../components/ui";

const GROUPS: [string, (t: Task, now: number) => boolean][] = [
  ["Overdue", (t, n) => !!t.due_at && new Date(t.due_at).getTime() < n],
  ["Today", (t, n) => !!t.due_at && sameDay(new Date(t.due_at), new Date(n))],
  ["Next 7 days", (t, n) => !!t.due_at && new Date(t.due_at).getTime() < n + 7 * 864e5],
  ["Later", (t) => !!t.due_at],
  ["No deadline", () => true],
];
function sameDay(a: Date, b: Date) { return a.toDateString() === b.toDateString(); }

const BOARD: [string, string[]][] = [
  ["To do", ["ready", "detected", "needs_confirmation"]],
  ["In progress", ["in_progress", "draft_prepared"]],
  ["Waiting", ["waiting", "blocked", "sent", "awaiting_approval"]],
  ["Done", ["completed"]],
];

export function Tasks({ open, search }: { open: (id: string) => void; search: string }) {
  const [view, setView] = useState<"list" | "board">(() => (localStorage.getItem("kept.view") as any) || "list");
  const [showClosed, setShowClosed] = useState(false);
  const [type, setType] = useState("");
  const [sort, setSort] = useState("due");
  const [adding, setAdding] = useState(false);
  const act = useAct();
  const [tasks] = useData<Task[]>(() => api.tasks({ include_closed: showClosed || view === "board", search, task_type: type || null, sort, top_level_only: true }), [showClosed, search, type, sort, view]);
  const setV = (v: "list" | "board") => { setView(v); try { localStorage.setItem("kept.view", v); } catch { /* ignore */ } };

  const groups = useMemo(() => {
    if (!tasks || sort !== "due") return null;
    const now = Date.now();
    const used = new Set<string>();
    return GROUPS.map(([name, fn]) => {
      const items = tasks.filter((t) => !used.has(t.id) && !["completed", "cancelled"].includes(t.status) && fn(t, now));
      items.forEach((t) => used.add(t.id));
      return [name, items] as const;
    }).concat([["Done", tasks.filter((t) => !used.has(t.id))] as const]).filter(([, i]) => i.length > 0);
  }, [tasks, sort]);

  return (
    <div className="page">
      <div className="page-head">
        <div><h1>My tasks</h1><div className="sub">Everything you've committed to, from every connected source.</div></div>
        <button className="btn primary" onClick={() => setAdding(true)}><Plus size={15} /> New task <span className="kbd" style={{ color: "inherit", borderColor: "rgba(255,255,255,.4)" }}>N</span></button>
      </div>
      <div className="btn-row" style={{ marginBottom: 14 }}>
        <div className="seg" role="group" aria-label="View">
          <button aria-pressed={view === "list"} onClick={() => setV("list")}>List</button>
          <button aria-pressed={view === "board"} onClick={() => setV("board")}>Board</button>
        </div>
        <select className="input" style={{ width: "auto" }} value={type} onChange={(e) => setType(e.target.value)} aria-label="Type">
          <option value="">All types</option>
          {Object.entries(TYPE_LABEL).map(([k, v]) => <option key={k} value={k}>{v}</option>)}
        </select>
        {view === "list" && (
          <>
            <select className="input" style={{ width: "auto" }} value={sort} onChange={(e) => setSort(e.target.value)} aria-label="Sort">
              <option value="due">By deadline</option><option value="priority">By priority</option><option value="created">Newest first</option><option value="updated">Recently updated</option>
            </select>
            <label className="toggle" style={{ padding: 0 }}><input type="checkbox" checked={showClosed} onChange={(e) => setShowClosed(e.target.checked)} /><span className="t-label">Show done</span></label>
          </>
        )}
        {search && <span className="muted small">Filtered by “{search}”</span>}
      </div>

      {!tasks ? <p className="muted">Loading…</p> : view === "board" ? (
        <div className="board">
          {BOARD.map(([name, sts]) => {
            const items = tasks.filter((t) => sts.includes(t.status));
            return (
              <div className="col" key={name}>
                <h3><span>{name}</span><span className="muted">{items.length}</span></h3>
                {items.slice(0, 60).map((t) => (
                  <div className="card" key={t.id} onClick={() => open(t.id)}>
                    <div className="t-title" style={{ fontSize: 13.5 }}>{t.title}</div>
                    <div className="t-meta">
                      {t.due_at && <span className={dueClass(t)}>{fmtDue(t.due_at)}</span>}
                      {name === "Waiting" && <span>{STATUS_LABEL[t.status]}</span>}
                    </div>
                    {name !== "Done" && (
                      <div className="btn-row" style={{ marginTop: 6 }}>
                        {BOARD.filter(([n]) => n !== name).map(([n, s]) => (
                          <button key={n} className="btn ghost sm" onClick={(e) => { e.stopPropagation(); act(() => api.setStatus(t.id, s[0]), `Moved to ${n}`); }}>{n}</button>
                        ))}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      ) : tasks.length === 0 ? (
        <div className="list"><Empty title={search ? "No tasks match your search." : "No open tasks."}>{!search && "Add one with New task, or connect an account in Integrations."}</Empty></div>
      ) : groups ? (
        groups.map(([name, items]) => (
          <div className="section" key={name}>
            <div className="section-head"><h2>{name}</h2><span className="muted small">{items.length}</span></div>
            <div className="list">{items.map((t) => <TaskRow key={t.id} t={t} onOpen={open} />)}</div>
          </div>
        ))
      ) : (
        <div className="list">{tasks.map((t) => <TaskRow key={t.id} t={t} onOpen={open} />)}</div>
      )}
      {adding && <NewTaskModal onClose={() => setAdding(false)} />}
    </div>
  );
}
