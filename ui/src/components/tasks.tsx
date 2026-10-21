import { useEffect, useState } from "react";
import { CalendarClock, ExternalLink, FileText, GitMerge, Link2, ListTree, RotateCcw, Send, Trash2, Users } from "lucide-react";
import { api, Doc, SourceItem, Task, TaskDetail, TaskPatch } from "../api";
import {
  CheckButton, Confidence, DUE_KIND_LABEL, Drawer, Modal, PRIORITY_LABEL, STATUS_LABEL, SourceNote, TYPE_LABEL,
  dueClass, fmtDue, fmtWhen, localInputValue, useAct,
} from "./ui";

const CLOSED = ["completed", "cancelled", "rejected"];

export function TaskRow({ t, onOpen, selected, showSource }: { t: Task; onOpen: (id: string) => void; selected?: boolean; showSource?: boolean }) {
  const act = useAct();
  const closed = CLOSED.includes(t.status);
  const who = t.people[0]?.name || t.people[0]?.email;
  return (
    <div className={`trow ${selected ? "selected" : ""}`} onClick={() => onOpen(t.id)} role="button" tabIndex={0} onKeyDown={(e) => e.key === "Enter" && onOpen(t.id)}>
      <CheckButton
        done={t.status === "completed"}
        onClick={() => act(() => api.setStatus(t.id, t.status === "completed" ? "ready" : "completed"), t.status === "completed" ? "Reopened" : "Marked done")}
      />
      <div style={{ minWidth: 0 }}>
        <div className={`t-title ${closed ? "closed" : ""}`}>{t.title}</div>
        <div className="t-meta">
          {t.status !== "ready" && <span className={`pill ${t.status === "awaiting_approval" || t.status === "waiting" ? "violet" : t.status === "blocked" ? "brick" : t.needs_confirmation ? "amber" : ""}`}>{STATUS_LABEL[t.status]}</span>}
          {who && <span><Users size={12} />{who}</span>}
          {t.meeting_title && <span><CalendarClock size={12} />{t.meeting_title}</span>}
          {t.project && <span>{t.project}</span>}
          {t.subtask_count > 0 && <span><ListTree size={12} />{t.subtasks_done}/{t.subtask_count}</span>}
          {t.origin === "external" && t.primary_source?.label && <span>{t.primary_source.label.split(" · ")[0]}</span>}
          {(t.origin === "ai" || t.origin === "rules") && <span title={`Detected by ${t.origin === "ai" ? "AI" : "built-in rules"}`}><Confidence value={t.confidence} /></span>}
          {t.source_count > 1 && <span><Link2 size={12} />{t.source_count} sources</span>}
        </div>
        {showSource && <SourceNote s={t.primary_source} compact />}
      </div>
      <div className="t-right">
        {t.due_at && <span className={dueClass(t)} title={t.due_kind === "inferred" ? "Inferred deadline" : "Deadline"}>{fmtDue(t.due_at)}</span>}
        {t.priority <= 2 && !closed && <span className={`pill ${t.priority === 1 ? "brick" : "amber"}`}>{PRIORITY_LABEL[t.priority]}</span>}
      </div>
    </div>
  );
}

export function TaskList({ tasks, onOpen, empty, showSource }: { tasks: Task[]; onOpen: (id: string) => void; empty?: React.ReactNode; showSource?: boolean }) {
  if (!tasks.length) return <div className="list">{empty}</div>;
  return <div className="list">{tasks.map((t) => <TaskRow key={t.id} t={t} onOpen={onOpen} showSource={showSource} />)}</div>;
}

const STATUS_FLOW = ["ready", "in_progress", "waiting", "blocked", "draft_prepared", "sent", "awaiting_approval", "completed", "cancelled"];

export function TaskDrawer({ id, onClose, onOpen }: { id: string; onClose: () => void; onOpen: (id: string) => void }) {
  const [d, setD] = useState<TaskDetail>();
  const [source, setSource] = useState<SourceItem | null>();
  const [edit, setEdit] = useState(false);
  const act = useAct();
  const load = () => api.task(id).then(setD).catch(() => onClose());
  useEffect(() => {
    load();
    const h = () => load();
    window.addEventListener("kept-changed", h);
    return () => window.removeEventListener("kept-changed", h);
  }, [id]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!d) return <Drawer onClose={onClose} head={<span className="muted">Loading…</span>}><span /></Drawer>;
  const t = d.task;
  const patch = (p: TaskPatch, msg = "Saved") => act(() => api.updateTask(t.id, p), msg).then(load);
  const status = (s: string) => act(() => api.setStatus(t.id, s), `Status: ${STATUS_LABEL[s]}`).then(load);
  const suggested = d.deadlines.find((x) => x.kind === "suggested" && !x.superseded_at);

  return (
    <Drawer
      onClose={onClose}
      head={
        <>
          <div className="t-meta" style={{ marginTop: 0, marginBottom: 6 }}>
            <span className="pill">{TYPE_LABEL[t.task_type] ?? t.task_type}</span>
            {(t.origin === "ai" || t.origin === "rules") && <span><Confidence value={t.confidence} /> {t.origin === "ai" ? "Found by AI" : "Found by built-in rules"}</span>}
            {t.origin === "external" && <span>Assigned to you in {t.primary_source?.label?.split(" · ")[0] ?? "a task tool"}</span>}
            {t.origin === "manual" && <span>Added by you</span>}
          </div>
          {edit ? (
            <input className="input" defaultValue={t.title} autoFocus onBlur={(e) => { setEdit(false); if (e.target.value.trim() && e.target.value !== t.title) patch({ title: e.target.value.trim() }); }} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
          ) : (
            <h2 style={{ fontSize: 19, margin: 0, cursor: "text" }} onClick={() => setEdit(true)} title="Click to edit">{t.title}</h2>
          )}
        </>
      }
    >
      {t.needs_confirmation && (
        <div className="banner">
          <div style={{ flex: 1 }}>
            <strong>Needs your review.</strong> {t.review_reason}
            <div className="rv-actions">
              <button className="btn sm primary" onClick={() => act(() => api.review("accept", [t.id]), "Accepted").then(load)}>Accept</button>
              <button className="btn sm" onClick={() => act(() => api.review("waiting", [t.id]), "Marked as waiting").then(load)}>Waiting on someone</button>
              <button className="btn sm danger" onClick={() => act(() => api.review("reject", [t.id]), "Rejected").then(onClose)}>Not a task</button>
            </div>
          </div>
        </div>
      )}
      {t.conflict_note && <div className="banner bad">{t.conflict_note}</div>}
      {d.possible_duplicate && (
        <div className="banner info">
          <GitMerge size={16} />
          <div style={{ flex: 1 }}>
            This may be the same as <a href="#" onClick={(e) => { e.preventDefault(); onOpen(d.possible_duplicate!.id); }}>{d.possible_duplicate.title}</a>.
            <div className="rv-actions">
              <button className="btn sm" onClick={() => act(() => api.review("merge", [t.id], d.possible_duplicate!.id), "Merged").then(() => onOpen(d.possible_duplicate!.id))}>Merge into it</button>
              <button className="btn sm ghost" onClick={() => act(() => api.review("accept", [t.id]), "Kept separate").then(load)}>Keep separate</button>
            </div>
          </div>
        </div>
      )}

      <div className="btn-row" style={{ marginBottom: 14 }}>
        <select className="input" style={{ width: "auto" }} value={t.status} onChange={(e) => status(e.target.value)} aria-label="Status">
          {STATUS_FLOW.concat(STATUS_FLOW.includes(t.status) ? [] : [t.status]).map((s) => <option key={s} value={s}>{STATUS_LABEL[s]}</option>)}
        </select>
        <select className="input" style={{ width: "auto" }} value={t.priority} onChange={(e) => patch({ priority: Number(e.target.value) })} aria-label="Priority">
          {[1, 2, 3, 4].map((p) => <option key={p} value={p}>{PRIORITY_LABEL[p]} priority</option>)}
        </select>
        <select className="input" style={{ width: "auto" }} value={t.my_role} onChange={(e) => patch({ my_role: e.target.value }, "Ownership updated")} aria-label="My role">
          <option value="owner">I own this</option>
          <option value="collaborator">I'm a collaborator</option>
          <option value="requester">Someone else owns it</option>
          <option value="mentioned">I'm only mentioned</option>
        </select>
      </div>

      {t.completion_evidence && (
        <div className={`note ${t.status === "completed" ? "" : "warn"}`} style={{ marginBottom: 14 }}>
          <span style={{ fontSize: 13 }}><strong>{t.status === "completed" ? "Completed because: " : "Progress: "}</strong>{t.completion_evidence}</span>
        </div>
      )}

      <dl className="kv">
        <dt>Deadline</dt>
        <dd>
          <input type="datetime-local" className="input" style={{ width: "auto" }} value={localInputValue(t.due_at)} onChange={(e) => patch({ due_at: e.target.value ? new Date(e.target.value).toISOString() : null }, "Deadline confirmed")} />
          {t.due_at && <button className="btn ghost sm" onClick={() => patch({ due_at: null }, "Deadline removed")}>Clear</button>}
          {t.due_at && <div className="small muted" style={{ marginTop: 4 }}>{t.due_phrase ? `"${t.due_phrase}" — ` : ""}{DUE_KIND_LABEL[t.due_kind]}</div>}
          {!t.due_at && suggested && (
            <div className="small" style={{ marginTop: 6 }}>
              No deadline was stated. Suggested: <strong>{fmtDue(suggested.due_at)}</strong>{" "}
              <button className="btn sm" onClick={() => patch({ due_at: suggested.due_at }, "Deadline confirmed")}>Use this</button>
            </div>
          )}
        </dd>
        <dt>Why this priority</dt><dd>{t.priority_reason || "—"}</dd>
        {t.meeting_title && <><dt>For meeting</dt><dd>{t.meeting_title}</dd></>}
        {t.people.length > 0 && <><dt>People</dt><dd>{t.people.map((p) => p.name || p.email).join(", ")}</dd></>}
        <dt>Deliverable</dt>
        <dd><input className="input" defaultValue={t.expected_deliverable ?? ""} placeholder="What will exist when this is done" onBlur={(e) => e.target.value !== (t.expected_deliverable ?? "") && patch({ expected_deliverable: e.target.value })} /></dd>
        <dt>Done when</dt>
        <dd><input className="input" defaultValue={t.completion_criteria ?? ""} placeholder="e.g. Report sent to Sara and approved" onBlur={(e) => e.target.value !== (t.completion_criteria ?? "") && patch({ completion_criteria: e.target.value })} /></dd>
        <dt>Project</dt>
        <dd><input className="input" defaultValue={t.project ?? ""} placeholder="Optional" onBlur={(e) => e.target.value !== (t.project ?? "") && patch({ project: e.target.value })} /></dd>
        <dt>Effort</dt>
        <dd>
          <select className="input" style={{ width: "auto" }} value={t.effort_minutes ?? 30} onChange={(e) => patch({ effort_minutes: Number(e.target.value) })}>
            {[10, 15, 30, 45, 60, 90, 120, 180, 240].map((m) => <option key={m} value={m}>{m < 60 ? `${m} min` : `${m / 60} h`}</option>)}
          </select>
        </dd>
        <dt>Repeats</dt>
        <dd>
          <select className="input" style={{ width: "auto" }} value={t.recurrence ?? ""} onChange={(e) => patch({ recurrence: e.target.value || null })}>
            <option value="">Doesn't repeat</option><option value="daily">Daily</option><option value="weekdays">Every weekday</option><option value="weekly">Weekly</option><option value="monthly">Monthly</option>
          </select>
        </dd>
      </dl>

      <label className="field" style={{ marginBottom: 18 }}>
        Notes
        <textarea className="input" defaultValue={t.description ?? ""} onBlur={(e) => e.target.value !== (t.description ?? "") && patch({ description: e.target.value })} />
      </label>

      <h3>Why this task exists</h3>
      {d.sources.length === 0 && <p className="muted small">Added manually.</p>}
      {d.sources.map((s) => (
        <div key={s.id} style={{ marginBottom: 10 }}>
          <SourceNote s={s} />
          <div className="btn-row" style={{ marginTop: 4 }}>
            {s.source_item_id && <button className="btn ghost sm" onClick={() => api.source(s.source_item_id!).then(setSource)}><FileText size={13} /> View message</button>}
            {s.url && <button className="btn ghost sm" onClick={() => act(() => api.openExternal(s.url!))}><ExternalLink size={13} /> Open original</button>}
          </div>
        </div>
      ))}

      <RelatedDocs taskId={t.id} />

      <Subtasks parent={t} items={d.subtasks} onOpen={onOpen} />

      {d.depends_on.length > 0 && (
        <>
          <h3 style={{ marginTop: 18 }}>Waiting on these first</h3>
          <div className="list">{d.depends_on.map((x) => <TaskRow key={x.id} t={x} onOpen={onOpen} />)}</div>
        </>
      )}

      {d.deadlines.length > 0 && (
        <>
          <h3 style={{ marginTop: 18 }}>Deadline history</h3>
          <div className="hist">
            {d.deadlines.slice().reverse().map((x) => (
              <div key={x.id} style={{ opacity: x.superseded_at ? 0.6 : 1 }}>
                <time>{fmtDue(x.due_at)}</time>
                <span>{x.kind}{x.phrase ? ` — "${x.phrase}"` : ""}{x.reason ? ` (${x.reason})` : ""}{x.superseded_at ? " · superseded" : ""}</span>
              </div>
            ))}
          </div>
        </>
      )}

      <h3 style={{ marginTop: 18 }}>History</h3>
      <div className="hist">
        {d.history.map((h) => (
          <div key={h.id}>
            <time>{fmtWhen(h.created_at)}</time>
            <span style={{ flex: 1 }}>
              {h.actor === "user" ? "You" : h.actor === "ai" ? "AI" : h.actor === "rules" ? "Rules" : "Kept"}: {h.action.replace("_", " ")}
              {h.to_value && h.action === "status" ? ` → ${STATUS_LABEL[h.to_value] ?? h.to_value}` : ""}
              {h.reason ? ` — ${h.reason}` : ""}
            </span>
            {!h.undone && ["status", "due", "owner", "priority", "title", "accepted"].includes(h.action) && (
              <button className="btn ghost sm" title="Undo this change" onClick={() => act(() => api.undo(h.id), "Undone").then(load)}><RotateCcw size={12} /></button>
            )}
            {h.undone && <span className="muted">undone</span>}
          </div>
        ))}
      </div>

      <div className="btn-row" style={{ marginTop: 24 }}>
        <SendToTodoist t={t} sources={d.sources} />
        <button className="btn danger sm" onClick={() => confirm("Delete this task permanently?") && act(() => api.deleteTask(t.id), "Deleted").then(onClose)}><Trash2 size={13} /> Delete</button>
      </div>

      {source && <SourceViewer item={source} onClose={() => setSource(undefined)} />}
    </Drawer>
  );
}

function Subtasks({ parent, items, onOpen }: { parent: Task; items: Task[]; onOpen: (id: string) => void }) {
  const [title, setTitle] = useState("");
  const act = useAct();
  return (
    <>
      <h3 style={{ marginTop: 18 }}>Subtasks</h3>
      {items.length > 0 && <div className="list" style={{ marginBottom: 8 }}>{items.map((s) => <TaskRow key={s.id} t={s} onOpen={onOpen} />)}</div>}
      <form className="btn-row" onSubmit={(e) => { e.preventDefault(); if (!title.trim()) return; act(() => api.createTask({ title: title.trim(), parent_id: parent.id, meeting_id: parent.meeting_id, due_at: parent.due_at })).then(() => setTitle("")); }}>
        <input className="input" style={{ flex: 1 }} placeholder="Add a subtask" value={title} onChange={(e) => setTitle(e.target.value)} />
        <button className="btn sm" type="submit">Add</button>
      </form>
    </>
  );
}

export function SourceViewer({ item, onClose }: { item: SourceItem; onClose: () => void }) {
  const act = useAct();
  return (
    <Modal onClose={onClose} title={item.subject || (item.kind === "chat" ? "Chat message" : "Email")}>
      <p className="small muted" style={{ marginTop: 0 }}>
        {item.is_from_me ? "You" : item.sender_name || item.sender_email} → {item.to.join(", ") || "channel"}
        {item.cc.length > 0 && ` (cc ${item.cc.join(", ")})`} — {new Date(item.sent_at).toLocaleString()}
      </p>
      <div className="note" style={{ whiteSpace: "pre-wrap", fontSize: 13.5 }}>
        {item.body_text ?? <span className="muted">The full text was removed by your retention setting. The quote on the task is kept.</span>}
      </div>
      <p className="small muted">Kept stores a bounded copy of the message for context. Treat links inside it with the same care as in your inbox.</p>
      <div className="btn-row">
        {item.url && <button className="btn" onClick={() => act(() => api.openExternal(item.url!))}><ExternalLink size={14} /> Open in {item.provider === "google" ? "Gmail" : item.provider === "microsoft" ? "Outlook" : "source app"}</button>}
        <button className="btn ghost" onClick={onClose}>Close</button>
      </div>
    </Modal>
  );
}

export function NewTaskModal({ onClose, defaults }: { onClose: () => void; defaults?: { meeting_id?: string } }) {
  const [title, setTitle] = useState("");
  const [due, setDue] = useState("");
  const [type, setType] = useState("action");
  const [priority, setPriority] = useState<string>("");
  const [notes, setNotes] = useState("");
  const act = useAct();
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!title.trim()) return;
    const r = await act(() => api.createTask({ title: title.trim(), task_type: type, due_at: due ? new Date(due).toISOString() : null, priority: priority ? Number(priority) : undefined, description: notes || null, meeting_id: defaults?.meeting_id }), "Task added");
    if (r !== undefined) onClose();
  };
  return (
    <Modal onClose={onClose} title="New task">
      <form onSubmit={submit} style={{ display: "grid", gap: 12 }}>
        <label className="field">What needs doing<input className="input" autoFocus value={title} onChange={(e) => setTitle(e.target.value)} placeholder="e.g. Send the Q3 forecast to finance" /></label>
        <div className="grid3">
          <label className="field">Deadline<input className="input" type="datetime-local" value={due} onChange={(e) => setDue(e.target.value)} /></label>
          <label className="field">Type
            <select className="input" value={type} onChange={(e) => setType(e.target.value)}>{Object.entries(TYPE_LABEL).map(([k, v]) => <option key={k} value={k}>{v}</option>)}</select>
          </label>
          <label className="field">Priority
            <select className="input" value={priority} onChange={(e) => setPriority(e.target.value)}><option value="">Automatic</option>{[1, 2, 3, 4].map((p) => <option key={p} value={p}>{PRIORITY_LABEL[p]}</option>)}</select>
          </label>
        </div>
        <label className="field">Notes<textarea className="input" value={notes} onChange={(e) => setNotes(e.target.value)} /></label>
        <div className="btn-row"><button className="btn primary" type="submit">Add task</button><button type="button" className="btn ghost" onClick={onClose}>Cancel</button></div>
      </form>
    </Modal>
  );
}

export function DocLinks({ docs, title = "Related documents" }: { docs: Doc[]; title?: string }) {
  const act = useAct();
  if (!docs.length) return null;
  return (
    <>
      <h3 style={{ marginTop: 18 }}>{title}</h3>
      <div className="list">
        {docs.map((d) => (
          <div key={d.id} className="trow" style={{ gridTemplateColumns: "22px 1fr auto" }} onClick={() => d.url && act(() => api.openExternal(d.url!))}>
            <FileText size={15} style={{ marginTop: 2, color: "var(--ink-3)" }} />
            <div>
              <div className="t-title" style={{ fontSize: 13.5 }}>{d.name}</div>
              <div className="t-meta"><span>{d.provider === "gdrive" ? "Google Drive" : "OneDrive / SharePoint"}</span>{d.modified_at && <span>edited {fmtWhen(d.modified_at)}{d.modified_by ? ` by ${d.modified_by}` : ""}</span>}</div>
            </div>
            <ExternalLink size={14} style={{ color: "var(--ink-3)", marginTop: 3 }} />
          </div>
        ))}
      </div>
      <p className="small muted">Matched by file name. Kept reads names and links only, never file contents.</p>
    </>
  );
}

function RelatedDocs({ taskId }: { taskId: string }) {
  const [docs, setDocs] = useState<Doc[]>([]);
  useEffect(() => { api.relatedDocuments(taskId).then(setDocs).catch(() => setDocs([])); }, [taskId]);
  return <DocLinks docs={docs} />;
}

function SendToTodoist({ t, sources }: { t: Task; sources: TaskDetail["sources"] }) {
  const [available, setAvailable] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const act = useAct();
  useEffect(() => { api.integrations().then((i) => setAvailable(i.accounts.some((a) => a.provider === "todoist" && a.status !== "attention"))).catch(() => {}); }, []);
  const already = sources.some((s) => s.provider === "todoist");
  if (!available || already || ["completed", "cancelled", "rejected"].includes(t.status)) return null;
  return (
    <>
      <button className="btn sm" onClick={() => setConfirming(true)}><Send size={13} /> Send to Todoist</button>
      {confirming && (
        <Modal onClose={() => setConfirming(false)} title="Create this task in Todoist?">
          <dl className="kv">
            <dt>Destination</dt><dd>Your Todoist inbox</dd>
            <dt>Task</dt><dd>{t.title}</dd>
            <dt>Due</dt><dd>{t.due_at ? fmtDue(t.due_at) : "No due date"}</dd>
            <dt>Effect</dt><dd>Creates one new task in Todoist. Nothing else changes. Kept links the two, so it won't appear twice.</dd>
          </dl>
          <div className="btn-row">
            <button className="btn primary" onClick={() => act(() => api.pushToTodoist(t.id), (m) => m).then(() => setConfirming(false))}>Create in Todoist</button>
            <button className="btn ghost" onClick={() => setConfirming(false)}>Cancel</button>
          </div>
        </Modal>
      )}
    </>
  );
}
