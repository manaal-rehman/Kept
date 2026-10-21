import { AlertTriangle, CalendarClock, PauseCircle } from "lucide-react";
import { api, Dashboard, Meeting } from "../api";
import { TaskList } from "../components/tasks";
import { Empty, SourceNote, fmtTime, fmtWhen, useAct, useData } from "../components/ui";

export function Home({ open, go }: { open: (id: string) => void; go: (v: string, arg?: string) => void }) {
  const [d] = useData<Dashboard>(api.dashboard);
  const act = useAct();
  if (!d) return <div className="page"><p className="muted">Loading…</p></div>;
  const n = d.next_up;
  const plan = d.plan;
  const total = Math.max(plan.available_minutes + plan.meeting_minutes, 1);

  return (
    <div className="page">
      {d.monitoring_paused && (
        <div className="banner"><PauseCircle size={16} /><div style={{ flex: 1 }}>Monitoring is paused. Kept isn't syncing or reminding you.</div><button className="btn sm" onClick={() => act(() => api.setPaused(false), "Monitoring resumed")}>Resume</button></div>
      )}
      {d.attention_accounts.length > 0 && (
        <div className="banner bad"><AlertTriangle size={16} /><div style={{ flex: 1 }}>{d.attention_accounts.join(", ")} need{d.attention_accounts.length === 1 ? "s" : ""} attention.</div><button className="btn sm" onClick={() => go("integrations")}>Open integrations</button></div>
      )}
      <div className="home">
        <div>
          <div className="greet">{d.greeting}</div>
          <div className="muted">{d.local_time}</div>
          <div className="briefing">{d.briefing.map((b, i) => <p key={i}>{b}</p>)}</div>

          {n && (
            <div className="nextup">
              <div className="lbl">Do this next</div>
              <div className="ttl" onClick={() => open(n.id)}>{n.title}</div>
              <div className="why">{n.priority_reason}</div>
              <SourceNote s={n.primary_source} compact />
              <div className="btn-row" style={{ marginTop: 12 }}>
                <button className="btn sm" onClick={() => act(() => api.setStatus(n.id, "in_progress"), "Started")}>Start</button>
                <button className="btn sm" onClick={() => act(() => api.setStatus(n.id, "completed"), "Marked done")}>Mark done</button>
                <button className="btn sm" onClick={() => open(n.id)}>Details</button>
              </div>
            </div>
          )}

          {d.overdue.length > 0 && (
            <div className="section">
              <div className="section-head"><h2>Overdue</h2><span className="muted small">{d.overdue.length}</span></div>
              <TaskList tasks={d.overdue} onOpen={open} />
            </div>
          )}
          <div className="section">
            <div className="section-head"><h2>Due today</h2></div>
            <TaskList tasks={d.due_today} onOpen={open} empty={<Empty title="Nothing else is due today." />} />
          </div>
          <div className="section">
            <div className="section-head"><h2>Priorities</h2><button className="btn ghost sm" onClick={() => go("tasks")}>All tasks</button></div>
            <TaskList tasks={d.priorities.filter((p) => p.id !== n?.id)} onOpen={open} empty={<Empty title="No open priorities." >Connect an account or add a task to get started.</Empty>} />
          </div>
          {d.review_count > 0 && (
            <div className="section">
              <div className="section-head"><h2>Waiting for your review</h2><button className="btn ghost sm" onClick={() => go("review")}>Review {d.review_count}</button></div>
              <TaskList tasks={d.review} onOpen={open} showSource />
            </div>
          )}
          {(d.waiting.length > 0 || d.blocked.length > 0) && (
            <div className="section">
              <div className="section-head"><h2>Waiting on others or blocked</h2><button className="btn ghost sm" onClick={() => go("followups")}>Follow-ups</button></div>
              <TaskList tasks={[...d.blocked, ...d.waiting]} onOpen={open} />
            </div>
          )}
          <div className="section">
            <div className="section-head"><h2>Recent activity</h2></div>
            <div className="hist" style={{ marginTop: 6 }}>
              {d.recent.length === 0 && <span className="muted">Nothing yet.</span>}
              {d.recent.map((r, i) => (
                <div key={i}><time>{fmtWhen(r.at)}</time><a href="#" onClick={(e) => { e.preventDefault(); r.task_id && open(r.task_id); }} style={{ color: "inherit" }}>{r.text}</a></div>
              ))}
            </div>
          </div>
        </div>

        <aside className="day">
          <div className="section">
            <div className="section-head"><h2>Today's plan</h2></div>
            {!plan.working_day ? (
              <p className="muted small">Not a working day. Nothing is scheduled.</p>
            ) : (
              <>
                <div className="meter" title="Meetings / planned work / free">
                  <b style={{ width: `${(plan.meeting_minutes / total) * 100}%` }} />
                  <b className={`t ${plan.overloaded ? "over" : ""}`} style={{ width: `${Math.min(plan.planned_minutes / total, 1) * 100}%` }} />
                </div>
                <div className="small muted" style={{ marginBottom: 10 }}>
                  {Math.round(plan.meeting_minutes / 6) / 10} h meetings · {Math.round(plan.planned_minutes / 6) / 10} h planned · {Math.round(plan.available_minutes / 6) / 10} h free
                </div>
                {plan.warnings.map((w, i) => <div key={i} className="banner" style={{ padding: "8px 10px", fontSize: 13 }}>{w}</div>)}
                <div className="timeline">
                  {plan.blocks.length === 0 && <p className="muted small">The rest of your working day is open.</p>}
                  {plan.blocks.map((b, i) => (
                    <div key={i} className={`tl-item ${b.kind}`} onClick={() => (b.task_id ? open(b.task_id) : b.meeting_id && go("meetings", b.meeting_id))}>
                      <span className="tl-time">{fmtTime(b.start)}</span>
                      <div className="tl-title">{b.title}</div>
                      <div className="tl-sub">{fmtTime(b.start)}–{fmtTime(b.end)}</div>
                    </div>
                  ))}
                </div>
              </>
            )}
          </div>
          <div className="section">
            <div className="section-head"><h2>Upcoming meetings</h2><button className="btn ghost sm" onClick={() => go("meetings")}>Meetings</button></div>
            <div className="list">
              {d.meetings.length === 0 && <Empty title="No meetings in the next 7 days." />}
              {d.meetings.map((m) => <MeetingLine key={m.id} m={m} onClick={() => go("meetings", m.id)} />)}
            </div>
          </div>
        </aside>
      </div>
    </div>
  );
}

export function MeetingLine({ m, onClick }: { m: Meeting; onClick: () => void }) {
  const day = new Date(m.start_at).toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" });
  return (
    <div className="trow" style={{ gridTemplateColumns: "22px 1fr" }} onClick={onClick} role="button" tabIndex={0}>
      <CalendarClock size={16} style={{ marginTop: 2, color: "var(--violet)" }} />
      <div>
        <div className="t-title">{m.title}</div>
        <div className="t-meta">
          <span>{day}, {fmtTime(m.start_at)}</span>
          {m.prep_state === "needed" && <span className="pill amber">Prep needed · {m.open_task_count}</span>}
          {m.prep_state === "unknown" && <span className="pill">No agenda</span>}
          {m.prep_state === "ready" && <span className="pill pine">Prepared</span>}
          {m.status === "cancelled" && <span className="pill brick">Cancelled</span>}
        </div>
      </div>
    </div>
  );
}
