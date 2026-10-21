import { useState } from "react";
import { BellOff, Clock, X } from "lucide-react";
import { api, Notification, Prefs, Rule } from "../api";
import { Empty, Toggle, fmtWhen, useAct, useData } from "../components/ui";

const CAT: Record<string, string> = {
  deadline: "Deadlines coming up", overdue: "Overdue work", meeting_prep: "Meetings that need preparation", review: "Items to review",
  followup: "Follow-ups and nudges", sync: "Integration problems", digest: "Daily digest and end-of-day review", critical: "Critical deadlines",
};
const STATE: Record<string, string> = { delivered: "Shown", in_app: "In Kept only", held: "Held (quiet hours or paused)", pending: "Queued", snoozed: "Snoozed", dismissed: "Dismissed", expired: "Expired", failed: "Couldn't be shown" };

export function Notifications({ open, go }: { open: (id: string) => void; go: (v: string, arg?: string) => void }) {
  const [tab, setTab] = useState("all");
  const [items] = useData<Notification[]>(() => api.notifications(tab), [tab]);
  const [prefs] = useData<Prefs>(api.prefs);
  const act = useAct();
  const paused = prefs?.notifications_paused_until && new Date(prefs.notifications_paused_until) > new Date();

  const openEntity = (n: Notification) => {
    act(() => api.notificationAction(n.id, "read"));
    if (n.entity_type === "task" && n.entity_id) open(n.entity_id);
    else if (n.entity_type === "meeting" && n.entity_id) go("meetings", n.entity_id);
    else if (n.entity_type === "account") go("integrations");
    else if (n.category === "review") go("review");
  };

  return (
    <div className="page">
      <div className="page-head">
        <div><h1>Notifications</h1><div className="sub">Every reminder Kept has shown or is holding. Dismissing or snoozing never changes the task.</div></div>
        <div className="btn-row">
          {paused ? (
            <button className="btn" onClick={() => act(() => api.setPaused(undefined, 0), "Notifications resumed")}>Resume notifications</button>
          ) : (
            <select className="input" style={{ width: "auto" }} value="" onChange={(e) => e.target.value && act(() => api.setPaused(undefined, Number(e.target.value)), "Notifications paused")} aria-label="Pause notifications">
              <option value="">Pause notifications…</option><option value="60">For 1 hour</option><option value="240">For 4 hours</option><option value="1440">Until tomorrow</option>
            </select>
          )}
          <button className="btn" onClick={() => act(() => api.testNotification(), "Test notification sent")}>Send a test</button>
          <button className="btn ghost" onClick={() => act(() => api.notificationAction(null, "read"), "All marked as read")}>Mark all read</button>
        </div>
      </div>
      {paused && <div className="banner"><BellOff size={16} /> Desktop notifications are paused until {new Date(prefs!.notifications_paused_until!).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}. They're kept here meanwhile.</div>}
      <div className="seg" role="group" style={{ marginBottom: 14 }}>
        {[["all", "History"], ["unread", "Unread"], ["pending", "Held or queued"], ["snoozed", "Snoozed"], ["rules", "Rules"]].map(([k, v]) => (
          <button key={k} aria-pressed={tab === k} onClick={() => setTab(k)}>{v}</button>
        ))}
      </div>
      {tab === "rules" ? <Rules /> : (
        <div className="list">
          {!items ? <p className="muted" style={{ padding: 16 }}>Loading…</p> : items.length === 0 ? <Empty title="Nothing here." /> : items.map((n) => (
            <div key={n.id} className="trow" style={{ gridTemplateColumns: "8px 1fr auto", opacity: ["dismissed", "expired"].includes(n.state) ? 0.6 : 1 }} onClick={() => openEntity(n)}>
              <span className="dot" style={{ background: n.read_at ? "transparent" : n.priority <= 1 ? "var(--brick)" : n.priority === 2 ? "var(--amber)" : "var(--pine-2)", marginTop: 6 }} />
              <div>
                <div className="t-title">{n.title}</div>
                <div className="small" style={{ color: "var(--ink-2)" }}>{n.body}</div>
                <div className="t-meta"><span>{fmtWhen(n.delivered_at ?? n.created_at)}</span><span>{STATE[n.state] ?? n.state}</span><span>{CAT[n.category] ?? n.category}</span>
                  {n.delivery_error && <span style={{ color: "var(--brick)" }}>{n.delivery_error}</span>}
                  {n.snoozed_until && n.state === "snoozed" && <span>until {new Date(n.snoozed_until).toLocaleString()}</span>}
                </div>
              </div>
              <div className="btn-row" onClick={(e) => e.stopPropagation()}>
                {!["dismissed", "expired"].includes(n.state) && (
                  <>
                    <select className="input" style={{ width: "auto", padding: "2px 6px", fontSize: 12.5 }} value="" onChange={(e) => e.target.value && act(() => api.notificationAction(n.id, "snooze", Number(e.target.value)), "Snoozed")} aria-label="Snooze">
                      <option value="">Snooze…</option><option value="30">30 min</option><option value="120">2 hours</option><option value="1080">Tomorrow</option>
                    </select>
                    <button className="btn ghost sm" title="Dismiss" onClick={() => act(() => api.notificationAction(n.id, "dismiss"))}><X size={14} /></button>
                  </>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function Rules() {
  const [rules, reload] = useData<Rule[]>(api.rules);
  const act = useAct();
  if (!rules) return null;
  const save = (r: Rule) => act(() => api.saveRule(r), "Rule saved").then(reload);
  return (
    <div className="panel">
      <p className="muted small" style={{ marginTop: 0 }}>
        <Clock size={13} style={{ verticalAlign: -2 }} /> “Lead time” is how early Kept warns you; “Repeat” re-reminds about overdue work. Only alerts at or above the priority you choose appear on the desktop; the rest stay in Kept. Quiet hours, the hourly limit and the digest are in Settings.
      </p>
      <table className="table">
        <thead><tr><th>Category</th><th>On</th><th>Desktop</th><th>Lead time (min)</th><th>Repeat (min)</th><th>Desktop if priority is</th><th>Ignore quiet hours</th></tr></thead>
        <tbody>
          {rules.map((r) => (
            <tr key={r.category}>
              <td>{CAT[r.category] ?? r.category}</td>
              <td><Toggle checked={r.enabled} onChange={(v) => save({ ...r, enabled: v })} label="" /></td>
              <td><Toggle checked={r.desktop} onChange={(v) => save({ ...r, desktop: v })} label="" /></td>
              <td><input className="input" type="number" min={0} defaultValue={r.lead_minutes} onBlur={(e) => Number(e.target.value) !== r.lead_minutes && save({ ...r, lead_minutes: Number(e.target.value) })} /></td>
              <td><input className="input" type="number" min={0} defaultValue={r.repeat_minutes} onBlur={(e) => Number(e.target.value) !== r.repeat_minutes && save({ ...r, repeat_minutes: Number(e.target.value) })} /></td>
              <td>
                <select className="input" style={{ width: "auto" }} value={r.min_priority} onChange={(e) => save({ ...r, min_priority: Number(e.target.value) })}>
                  <option value={1}>Critical only</option><option value={2}>High or above</option><option value={3}>Normal or above</option><option value={4}>Any</option>
                </select>
              </td>
              <td><Toggle checked={r.bypass_quiet} onChange={(v) => save({ ...r, bypass_quiet: v })} label="" /></td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
