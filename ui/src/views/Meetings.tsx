import { useEffect, useState } from "react";
import { ExternalLink, Plus, Sparkles, Video } from "lucide-react";
import { api, Briefing, Meeting } from "../api";
import { DocLinks, NewTaskModal, SourceViewer, TaskList } from "../components/tasks";
import { Empty, fmtDay, fmtTime, useAct, useData } from "../components/ui";

export function Meetings({ open, initial }: { open: (id: string) => void; initial?: string }) {
  const [ms] = useData<Meeting[]>(() => api.meetings(21));
  const [sel, setSel] = useState<string | undefined>(initial);
  useEffect(() => { if (!sel && ms?.length) setSel((ms.find((m) => m.prep_state === "needed") ?? ms[0]).id); }, [ms, sel]);
  useEffect(() => { if (initial) setSel(initial); }, [initial]);
  if (!ms) return <div className="page"><p className="muted">Loading…</p></div>;
  const byDay = new Map<string, Meeting[]>();
  for (const m of ms) {
    const k = fmtDay(m.start_at);
    byDay.set(k, [...(byDay.get(k) ?? []), m]);
  }
  return (
    <div className="page">
      <div className="page-head"><div><h1>Meetings</h1><div className="sub">The next three weeks, with what you need to prepare.</div></div></div>
      {ms.length === 0 ? (
        <div className="list"><Empty title="No upcoming meetings.">Connect Google Calendar or Microsoft 365, or import an .ics file in Integrations.</Empty></div>
      ) : (
        <div className="mt-layout">
          <div>
            {[...byDay.entries()].map(([day, items]) => (
              <div key={day}>
                <div className="mt-day">{day}</div>
                {items.map((m) => (
                  <div key={m.id} className="mt" aria-current={m.id === sel} onClick={() => setSel(m.id)} role="button" tabIndex={0} onKeyDown={(e) => e.key === "Enter" && setSel(m.id)}>
                    <div className="t-title" style={{ textDecoration: m.status === "cancelled" ? "line-through" : undefined }}>{m.title}</div>
                    <div className="t-meta">
                      <span>{m.all_day ? "All day" : `${fmtTime(m.start_at)}–${fmtTime(m.end_at)}`}</span>
                      {m.prep_state === "needed" && <span className="pill amber">Prep: {m.open_task_count} open</span>}
                      {m.prep_state === "unknown" && <span className="pill">No agenda</span>}
                      {m.prep_state === "ready" && <span className="pill pine">Prepared</span>}
                    </div>
                  </div>
                ))}
              </div>
            ))}
          </div>
          <div>{sel && <BriefingPanel id={sel} open={open} />}</div>
        </div>
      )}
    </div>
  );
}

function BriefingPanel({ id, open }: { id: string; open: (id: string) => void }) {
  const [b, reload] = useData<Briefing>(() => api.briefing(id), [id]);
  const [adding, setAdding] = useState(false);
  const [busy, setBusy] = useState(false);
  const [viewer, setViewer] = useState<any>();
  const act = useAct();
  if (!b) return <div className="panel"><p className="muted">Loading briefing…</p></div>;
  const m = b.meeting;
  return (
    <div className="panel">
      <h2 style={{ fontSize: 20, marginBottom: 4 }}>{m.title}</h2>
      <div className="muted">{b.local_start} · {b.duration_minutes} min{m.location ? ` · ${m.location}` : ""}</div>
      <div className="btn-row" style={{ margin: "12px 0 4px" }}>
        {m.join_url && <button className="btn sm" onClick={() => act(() => api.openExternal(m.join_url!))}><Video size={14} /> Join link</button>}
        {m.url && <button className="btn sm ghost" onClick={() => act(() => api.openExternal(m.url!))}><ExternalLink size={14} /> Open in calendar</button>}
        <button className="btn sm ghost" onClick={() => setAdding(true)}><Plus size={14} /> Add prep task</button>
        <button className="btn sm ghost" disabled={busy} onClick={async () => { setBusy(true); await act(() => api.aiBriefing(id)); setBusy(false); reload(); }}><Sparkles size={14} /> {busy ? "Summarising…" : "AI summary"}</button>
      </div>
      {b.notes.map((n, i) => <div key={i} className="banner" style={{ marginTop: 10 }}>{n}</div>)}
      {b.ai_summary && (
        <div className="note ai" style={{ margin: "12px 0" }}>
          <span style={{ fontSize: 13.5 }}>{b.ai_summary}</span>
          <span className="from">AI summary of the facts below. Check anything important against the sources.</span>
        </div>
      )}

      <h3 style={{ marginTop: 16 }}>Preparation checklist</h3>
      {b.checklist.length === 0 ? <p className="muted small">Nothing specific to prepare was found. That doesn't mean nothing is needed — Kept only lists what it can point to.</p> : (
        <div>
          {b.checklist.map((c, i) => (
            <div className="check-item" key={i}>
              <input type="checkbox" checked={c.done} disabled={!c.task_id} onChange={() => c.task_id && act(() => api.setStatus(c.task_id!, c.done ? "ready" : "completed"))} aria-label="Done" />
              <div style={{ flex: 1 }}>
                <a href="#" style={{ color: "inherit" }} onClick={(e) => { e.preventDefault(); if (c.task_id) open(c.task_id); else if (c.source_item_id) api.source(c.source_item_id).then(setViewer); }}>{c.text}</a>
                <div className="small muted">
                  {c.basis === "task" ? "From a tracked task" : c.basis === "source" ? "From a message" : "Suggestion (not from a source)"}
                  {c.due_before_start ? " · before the meeting starts" : ""}
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {b.agenda && (<><h3 style={{ marginTop: 18 }}>Agenda</h3><div style={{ whiteSpace: "pre-wrap", fontSize: 13.5 }}>{b.agenda}</div></>)}

      <h3 style={{ marginTop: 18 }}>Attendees</h3>
      <div className="small">{m.attendees.map((a) => `${a.name || a.email}${a.response && a.response !== "accepted" ? ` (${a.response})` : ""}`).join(", ") || "Just you"}</div>

      {b.linked_tasks.length > 0 && (<><h3 style={{ marginTop: 18 }}>Tasks for this meeting</h3><TaskList tasks={b.linked_tasks} onOpen={open} /></>)}
      {b.promises_to_attendees.length > 0 && (<><h3 style={{ marginTop: 18 }}>Your open promises to attendees</h3><TaskList tasks={b.promises_to_attendees} onOpen={open} /></>)}
      {b.waiting_on_attendees.length > 0 && (
        <><h3 style={{ marginTop: 18 }}>Attendees owe you</h3>
          <ul className="small">{b.waiting_on_attendees.map((c) => <li key={c.id}>{c.text}</li>)}</ul></>
      )}
      {b.previous_followups.length > 0 && (<><h3 style={{ marginTop: 18 }}>Open follow-ups from earlier sessions</h3><TaskList tasks={b.previous_followups} onOpen={open} /></>)}
      {b.related_messages.length > 0 && (
        <>
          <h3 style={{ marginTop: 18 }}>Related conversations</h3>
          {b.related_messages.map((r) => (
            <div key={r.source_item_id} className="note" style={{ cursor: "pointer" }} onClick={() => api.source(r.source_item_id).then(setViewer)}>
              <span style={{ fontSize: 13.5 }}><strong>{r.subject || "Message"}</strong> — {r.snippet}</span>
              <span className="from">{r.label}</span>
            </div>
          ))}
        </>
      )}
      <DocLinks docs={b.related_documents} title="Documents that may be relevant" />
      <p className="small muted" style={{ marginTop: 18 }}>Kept reads calendar details and your synced messages only. It cannot read meeting recordings or transcripts.</p>
      {adding && <NewTaskModal onClose={() => setAdding(false)} defaults={{ meeting_id: m.id }} />}
      {viewer && <SourceViewer item={viewer} onClose={() => setViewer(undefined)} />}
    </div>
  );
}
