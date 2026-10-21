import { api, Followups as F } from "../api";
import { TaskList } from "../components/tasks";
import { Empty, fmtDue, useAct, useData } from "../components/ui";

export function Followups({ open }: { open: (id: string) => void }) {
  const [f] = useData<F>(api.followups);
  const act = useAct();
  if (!f) return <div className="page"><p className="muted">Loading…</p></div>;
  const Sec = ({ title, sub, children, n }: { title: string; sub: string; children: React.ReactNode; n: number }) => (
    <div className="section">
      <div className="section-head"><h2>{title}</h2><span className="muted small">{n}</span></div>
      <p className="muted small" style={{ margin: "-4px 0 8px" }}>{sub}</p>
      {children}
    </div>
  );
  return (
    <div className="page">
      <div className="page-head"><div><h1>Follow-ups</h1><div className="sub">Loops that close only when someone replies, approves, or delivers.</div></div></div>
      <Sec title="Replies you owe" sub="Direct questions and requests for your answer." n={f.awaiting_my_reply.length}>
        <TaskList tasks={f.awaiting_my_reply} onOpen={open} empty={<Empty title="No replies owed." />} />
      </Sec>
      <Sec title="People you said you'd get back to" sub="“Let me check and get back to you” and similar promises." n={f.people_to_contact.length}>
        <TaskList tasks={f.people_to_contact} onOpen={open} empty={<Empty title="Nobody is waiting on a call-back." />} />
      </Sec>
      <Sec title="Awaiting approval" sub="You sent something; it isn't done until it's approved." n={f.awaiting_approval.length}>
        <TaskList tasks={f.awaiting_approval} onOpen={open} empty={<Empty title="Nothing pending approval." />} />
      </Sec>
      <Sec title="Others owe you" sub="Promises other people made to you. These never become your tasks." n={f.waiting_on_others.length + f.waiting_tasks.length}>
        <div className="list">
          {f.waiting_on_others.length + f.waiting_tasks.length === 0 && <Empty title="You're not waiting on anyone." />}
          {f.waiting_on_others.map((c) => (
            <div className="trow" key={c.id} style={{ gridTemplateColumns: "1fr auto", cursor: "default" }}>
              <div>
                <div className="t-title">{c.text}</div>
                <div className="t-meta"><span>{c.source_label}</span>{c.due_at && <span className="due">{fmtDue(c.due_at)}</span>}</div>
              </div>
              <div className="btn-row">
                <button className="btn sm" onClick={() => act(() => api.setCommitment(c.id, "fulfilled"), "Marked as received")}>Received</button>
                <button className="btn sm ghost" onClick={() => act(() => api.setCommitment(c.id, "dismissed"), "Dismissed")}>Dismiss</button>
              </div>
            </div>
          ))}
        </div>
        {f.waiting_tasks.length > 0 && <div style={{ marginTop: 8 }}><TaskList tasks={f.waiting_tasks} onOpen={open} /></div>}
      </Sec>
      <Sec title="Blocked by others" sub="Tasks you marked as blocked." n={f.blocked_by_others.length}>
        <TaskList tasks={f.blocked_by_others} onOpen={open} empty={<Empty title="Nothing blocked." />} />
      </Sec>
      <Sec title="Promises past due with no evidence" sub="Detected commitments whose deadline passed and that Kept has no sign you delivered." n={f.promises_without_evidence.length}>
        <TaskList tasks={f.promises_without_evidence} onOpen={open} showSource empty={<Empty title="No forgotten promises." />} />
      </Sec>
    </div>
  );
}
