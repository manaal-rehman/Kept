import { useEffect, useState } from "react";
import { api, Prefs } from "../api";
import { Modal, Toggle, fmtWhen, useAct, useData } from "../components/ui";

const SECTIONS = [["general", "General"], ["hours", "Working hours"], ["ai", "AI provider"], ["privacy", "Privacy and data"], ["background", "Background and startup"], ["diagnostics", "Diagnostics"]];
const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

function zones(): string[] {
  try { return (Intl as any).supportedValuesOf("timeZone"); } catch { return ["UTC"]; }
}

export function Settings() {
  const [saved, reload] = useData<Prefs>(api.prefs);
  const [p, setP] = useState<Prefs>();
  const [sec, setSec] = useState("general");
  const act = useAct();
  useEffect(() => { if (saved) setP(saved); }, [saved]);
  if (!p) return <div className="page"><p className="muted">Loading…</p></div>;
  const dirty = JSON.stringify(p) !== JSON.stringify(saved);
  const set = <K extends keyof Prefs>(k: K, v: Prefs[K]) => setP({ ...p, [k]: v });
  const setAi = (k: keyof Prefs["ai"], v: any) => setP({ ...p, ai: { ...p.ai, [k]: v } });
  const save = () => act(() => api.savePrefs(p), "Settings saved").then(reload);
  const list = (s: string) => s.split(/[,\n]/).map((x) => x.trim()).filter(Boolean);

  return (
    <div className="page">
      <div className="page-head">
        <div><h1>Settings</h1></div>
        <div className="btn-row">{dirty && <><button className="btn ghost" onClick={() => setP(saved)}>Discard</button><button className="btn primary" onClick={save}>Save changes</button></>}</div>
      </div>
      <div className="settings-nav seg">{SECTIONS.map(([k, v]) => <button key={k} aria-pressed={sec === k} onClick={() => setSec(k)}>{v}</button>)}</div>

      {sec === "general" && (
        <div className="panel">
          <div className="grid2">
            <label className="field">Your name<input className="input" value={p.user_name} onChange={(e) => set("user_name", e.target.value)} /></label>
            <label className="field">Other names or handles <span className="hint">Nicknames and chat handles people use for you, comma separated.</span><input className="input" value={p.user_aliases.join(", ")} onChange={(e) => set("user_aliases", list(e.target.value))} /></label>
          </div>
          <label className="field" style={{ marginTop: 14 }}>Your email addresses <span className="hint">Every address that is “you”. Kept uses these to tell your promises from other people's. Connected accounts are added automatically.</span>
            <textarea className="input" value={p.user_emails.join("\n")} onChange={(e) => set("user_emails", list(e.target.value))} />
          </label>
          <label className="field" style={{ marginTop: 14 }}>Important people <span className="hint">Email addresses whose requests get extra priority (manager, key clients).</span>
            <textarea className="input" value={p.vip_emails.join("\n")} onChange={(e) => set("vip_emails", list(e.target.value))} />
          </label>
          <h3 style={{ marginTop: 18 }}>Detected tasks</h3>
          <Toggle checked={p.review_everything} onChange={(v) => set("review_everything", v)} label="Review every detected task" desc="Off: high-confidence tasks that are clearly yours go straight to your list. Everything uncertain always goes to Review." />
          <label className="field" style={{ maxWidth: 360, marginTop: 8 }}>Confidence needed to skip review: {Math.round(p.auto_accept_threshold * 100)}%
            <input type="range" min={60} max={100} value={Math.round(p.auto_accept_threshold * 100)} onChange={(e) => set("auto_accept_threshold", Number(e.target.value) / 100)} />
          </label>
        </div>
      )}

      {sec === "hours" && (
        <div className="panel">
          <div className="grid3">
            <label className="field">Timezone<select className="input" value={p.timezone} onChange={(e) => set("timezone", e.target.value)}>{zones().map((z) => <option key={z}>{z}</option>)}</select></label>
            <label className="field">Workday starts<input className="input" type="time" value={p.work_start} onChange={(e) => set("work_start", e.target.value)} /></label>
            <label className="field">Workday ends<input className="input" type="time" value={p.work_end} onChange={(e) => set("work_end", e.target.value)} /></label>
          </div>
          <div style={{ marginTop: 14 }}>
            <div className="field" style={{ marginBottom: 6 }}>Working days</div>
            <div className="seg">{DAYS.map((d, i) => <button key={d} aria-pressed={p.working_days.includes(i + 1)} onClick={() => set("working_days", p.working_days.includes(i + 1) ? p.working_days.filter((x) => x !== i + 1) : [...p.working_days, i + 1].sort())}>{d}</button>)}</div>
          </div>
          <h3 style={{ marginTop: 18 }}>Quiet hours</h3>
          <Toggle checked={p.quiet_hours_enabled} onChange={(v) => set("quiet_hours_enabled", v)} label="Hold notifications during quiet hours" desc="They're delivered when quiet hours end. Critical alerts can be allowed through in notification rules." />
          <div className="grid3">
            <label className="field">From<input className="input" type="time" value={p.quiet_start} onChange={(e) => set("quiet_start", e.target.value)} /></label>
            <label className="field">Until<input className="input" type="time" value={p.quiet_end} onChange={(e) => set("quiet_end", e.target.value)} /></label>
            <label className="field">At most this many alerts per hour<input className="input" type="number" min={1} max={60} value={p.max_notifications_per_hour} onChange={(e) => set("max_notifications_per_hour", Number(e.target.value))} /></label>
          </div>
          <h3 style={{ marginTop: 18 }}>Daily rhythm</h3>
          <div className="grid2">
            <div><Toggle checked={p.digest_enabled} onChange={(v) => set("digest_enabled", v)} label="Morning digest" /><input className="input" type="time" value={p.digest_time} onChange={(e) => set("digest_time", e.target.value)} style={{ maxWidth: 140 }} /></div>
            <div><Toggle checked={p.eod_review_enabled} onChange={(v) => set("eod_review_enabled", v)} label="End-of-day review" /><input className="input" type="time" value={p.eod_review_time} onChange={(e) => set("eod_review_time", e.target.value)} style={{ maxWidth: 140 }} /></div>
          </div>
          <div className="grid2" style={{ marginTop: 14 }}>
            <label className="field">Planning style
              <select className="input" value={p.planning_style} onChange={(e) => set("planning_style", e.target.value)}>
                <option value="balanced">Balanced — priority, then deadline</option><option value="deadline_first">Deadline first</option><option value="quick_wins">Quick wins first</option>
              </select>
            </label>
            <label className="field">Finish meeting prep this long before it starts (min)<input className="input" type="number" min={0} max={240} value={p.meeting_prep_buffer_minutes} onChange={(e) => set("meeting_prep_buffer_minutes", Number(e.target.value))} /></label>
          </div>
          <label className="field" style={{ marginTop: 14 }}>Holidays <span className="hint">Dates with no planning or digest, one per line (YYYY-MM-DD).</span>
            <textarea className="input" value={p.holidays.join("\n")} onChange={(e) => set("holidays", list(e.target.value).filter((d) => /^\d{4}-\d{2}-\d{2}$/.test(d)))} />
          </label>
        </div>
      )}

      {sec === "ai" && <AiSection p={p} set={set} setAi={setAi} save={save} dirty={dirty} />}

      {sec === "privacy" && <PrivacySection p={p} set={set} setAi={setAi} />}

      {sec === "background" && (
        <div className="panel">
          <Toggle checked={p.autostart} onChange={(v) => set("autostart", v)} label="Start Kept when I sign in to Windows" desc="Kept starts minimised in the tray so reminders work from the moment you log in." />
          <Toggle checked={p.start_minimized} onChange={(v) => set("start_minimized", v)} label="Start minimised to the tray" />
          <Toggle checked={p.close_to_tray} onChange={(v) => set("close_to_tray", v)} label="Keep running in the tray when I close the window" desc="Recommended. Closing the window frees its memory; the agent keeps watching. Turn off to quit when you close the window." />
          <Toggle checked={p.monitoring_paused} onChange={(v) => set("monitoring_paused", v)} label="Pause monitoring" desc="Stop syncing and analysing until you resume. Your tasks stay available." />
          <div className="grid2" style={{ marginTop: 10 }}>
            <label className="field">Check for new messages every (minutes)<input className="input" type="number" min={2} max={240} value={p.sync_interval_minutes} onChange={(e) => set("sync_interval_minutes", Number(e.target.value))} /></label>
            <label className="field">On first connection, read the last (days)<input className="input" type="number" min={1} max={90} value={p.initial_sync_days} onChange={(e) => set("initial_sync_days", Number(e.target.value))} /></label>
          </div>
          <p className="small muted">When your laptop is asleep, off or offline Kept can't check anything. When it wakes up, it syncs what it missed and recalculates reminders, skipping alerts that are no longer useful.</p>
        </div>
      )}

      {sec === "diagnostics" && <Diagnostics />}
      {dirty && sec !== "ai" && <div className="btn-row" style={{ marginTop: 16 }}><button className="btn primary" onClick={save}>Save changes</button></div>}
    </div>
  );
}

function AiSection({ p, set, setAi, save, dirty }: { p: Prefs; set: any; setAi: (k: any, v: any) => void; save: () => Promise<unknown>; dirty: boolean }) {
  const [key, setKey] = useState("");
  const [has, setHas] = useState(false);
  const act = useAct();
  const kind = `ai:${p.ai.provider}`;
  useEffect(() => { if (p.ai.provider !== "none") api.hasSecret(kind).then(setHas); }, [kind, p.ai.provider]);
  const models: Record<string, string> = { anthropic: "claude-haiku-5-5", openai: "gpt-4.1-mini", openai_compatible: "llama3.1" };
  return (
    <div className="panel">
      <p style={{ marginTop: 0, maxWidth: "70ch" }}>
        AI makes detection much better at unusual wording. Without it Kept uses built-in rules, which work offline and rarely invent tasks but miss more.
        AI is billed by the provider per use through an <strong>API key</strong> — that's separate from a Claude.ai or ChatGPT subscription, which does not include API access. Typical use costs a few cents to a couple of dollars a month with a small model.
      </p>
      <div className="grid2">
        <label className="field">Provider
          <select className="input" value={p.ai.provider} onChange={(e) => setAi("provider", e.target.value)}>
            <option value="none">None — built-in rules only</option>
            <option value="anthropic">Anthropic (Claude)</option>
            <option value="openai">OpenAI</option>
            <option value="openai_compatible">OpenAI-compatible (Ollama, LM Studio, OpenRouter…)</option>
          </select>
        </label>
        {p.ai.provider !== "none" && <label className="field">Model <span className="hint">Leave blank for the default ({models[p.ai.provider]}). Check your provider's current model names.</span><input className="input" value={p.ai.model} placeholder={models[p.ai.provider]} onChange={(e) => setAi("model", e.target.value)} /></label>}
      </div>
      {p.ai.provider === "openai_compatible" && <label className="field" style={{ marginTop: 12 }}>Base URL<input className="input" value={p.ai.base_url} placeholder="http://localhost:11434/v1" onChange={(e) => setAi("base_url", e.target.value)} /></label>}
      {p.ai.provider !== "none" && (
        <label className="field" style={{ marginTop: 12 }}>API key <span className="hint">{has ? "A key is saved in Windows Credential Manager. Enter a new one to replace it." : "Stored only in Windows Credential Manager — never in files, logs, or this window after you save it."}</span>
          <div className="btn-row">
            <input className="input" style={{ flex: 1 }} type="password" autoComplete="off" value={key} onChange={(e) => setKey(e.target.value)} placeholder={has ? "••••••••" : p.ai.provider === "anthropic" ? "sk-ant-…" : "sk-…"} />
            <button className="btn" disabled={!key} onClick={() => act(() => api.setSecret(kind, key), "Key saved").then(() => { setKey(""); setHas(true); })}>Save key</button>
            {has && <button className="btn ghost" onClick={() => act(() => api.setSecret(kind, ""), "Key removed").then(() => setHas(false))}>Remove</button>}
          </div>
        </label>
      )}
      <div className="grid2" style={{ marginTop: 14 }}>
        <label className="field">Detection mode
          <select className="input" value={p.extraction_mode} onChange={(e) => set("extraction_mode", e.target.value)}>
            <option value="auto">Use AI when configured, rules otherwise</option><option value="rules_only">Never use AI</option>
          </select>
        </label>
        <label className="field">Daily AI request limit<input className="input" type="number" min={0} value={p.ai.daily_request_limit} onChange={(e) => setAi("daily_request_limit", Number(e.target.value))} /></label>
      </div>
      <div className="btn-row" style={{ marginTop: 16 }}>
        {dirty && <button className="btn primary" onClick={save}>Save changes</button>}
        {p.ai.provider !== "none" && <button className="btn" disabled={dirty} title={dirty ? "Save first" : ""} onClick={() => act(() => api.testAi(), (m) => m)}>Test connection</button>}
      </div>
    </div>
  );
}

function PrivacySection({ p, set, setAi }: { p: Prefs; set: any; setAi: (k: any, v: any) => void }) {
  const [info] = useData<any>(api.storage);
  const [confirmDel, setConfirmDel] = useState(false);
  const [typed, setTyped] = useState("");
  const act = useAct();
  return (
    <>
      <div className="panel">
        <h3>What leaves your computer</h3>
        <p className="small" style={{ maxWidth: "70ch" }}>Kept talks only to the services you connect and, if you configure one, your AI provider. For AI it sends the newly written part of one message at a time (quoted history removed), a few short excerpts of earlier messages in the thread, and titles of possibly related tasks — never your whole mailbox. Passwords, API keys and tokens are always stripped. There's no Kept server, telemetry or analytics.</p>
        <Toggle checked={p.ai.process_email} onChange={(v) => setAi("process_email", v)} label="Let AI read emails" />
        <Toggle checked={p.ai.process_chat} onChange={(v) => setAi("process_chat", v)} label="Let AI read chat messages" />
        <Toggle checked={p.ai.process_calendar} onChange={(v) => setAi("process_calendar", v)} label="Let AI summarise meeting briefings" />
        <Toggle checked={p.ai.redact_pii} onChange={(v) => setAi("redact_pii", v)} label="Hide email addresses and phone numbers from AI" desc="Names stay so the AI can tell who asked whom." />
        <label className="field" style={{ maxWidth: 360, marginTop: 8 }}>Most text per AI request (characters)<input className="input" type="number" min={1000} max={40000} step={500} value={p.ai.max_chars_per_request} onChange={(e) => setAi("max_chars_per_request", Number(e.target.value))} /></label>
      </div>
      <div className="panel">
        <h3>What's stored here</h3>
        <p className="small" style={{ maxWidth: "70ch" }}>Everything lives in a database in your Windows profile, protected by your Windows account. It is not separately encrypted — use BitLocker or device encryption to protect it if your laptop is lost. Sign-ins and keys are in Windows Credential Manager.</p>
        <div className="grid2">
          <label className="field">Delete message text after (days)<input className="input" type="number" min={1} max={365} value={p.retention_body_days} onChange={(e) => set("retention_body_days", Number(e.target.value))} /></label>
          <label className="field">Forget synced messages entirely after (days)<input className="input" type="number" min={7} max={730} value={p.retention_item_days} onChange={(e) => set("retention_item_days", Number(e.target.value))} /></label>
        </div>
        {info && <p className="small muted">{(info.database_bytes / 1048576).toFixed(1)} MB · {info.counts.tasks} tasks · {info.counts.source_items} messages · {info.counts.meetings} meetings · AI: {info.ai.requests_30d} requests ({Math.round(info.ai.chars_sent_30d / 1000)}k characters) in the last 30 days · {info.data_dir}</p>}
        <div className="btn-row">
          <button className="btn" onClick={() => act(() => api.exportData(), (f) => (f ? `Exported to ${f}` : undefined))}>Export my data (JSON)</button>
          <button className="btn" onClick={() => act(() => api.backupData(), (f) => (f ? `Backup saved to ${f}` : undefined))}>Back up database</button>
          <button className="btn danger" onClick={() => setConfirmDel(true)}>Delete all local data</button>
        </div>
      </div>
      {confirmDel && (
        <Modal title="Delete everything Kept has stored?" onClose={() => setConfirmDel(false)}>
          <p>This removes all tasks, synced messages, meetings, notifications, settings and history from this computer, and deletes saved sign-ins and API keys from Credential Manager. It can't be undone. Your email and calendars are not touched.</p>
          <label className="field">Type DELETE to confirm<input className="input" value={typed} onChange={(e) => setTyped(e.target.value)} /></label>
          <div className="btn-row" style={{ marginTop: 14 }}>
            <button className="btn danger" disabled={typed !== "DELETE"} onClick={() => act(() => api.deleteAll(typed), "All local data deleted").then(() => location.reload())}>Delete everything</button>
            <button className="btn ghost" onClick={() => setConfirmDel(false)}>Cancel</button>
          </div>
        </Modal>
      )}
    </>
  );
}

function Diagnostics() {
  const [info] = useData<any>(api.appInfo);
  const [log] = useData<any[]>(() => api.auditLog(60));
  const [ai] = useData<any[]>(api.aiUsage);
  const act = useAct();
  return (
    <>
      <div className="panel">
        {info && <p className="small" style={{ marginTop: 0 }}>Kept {info.version} on {info.os} · starts with Windows: {info.autostart ? "yes" : "no"} · credential store: {info.keychain_ok ? "working" : "unavailable"}</p>}
        <div className="btn-row">
          <button className="btn" onClick={() => act(() => api.openFolder("logs"))}>Open log folder</button>
          <button className="btn" onClick={() => act(() => api.openFolder("data"))}>Open data folder</button>
          <button className="btn" onClick={() => act(() => api.reanalyse(14), (m) => m)}>Re-check recent messages</button>
        </div>
        <p className="small muted">“Re-check” looks again at messages from the last 14 days that produced no task — useful after adding an AI key or updating Kept. Messages you already acted on (including rejected tasks) are not touched.</p>
        <p className="small muted">Logs record what Kept did (sync counts, errors) — never message text, tokens or keys.</p>
      </div>
      <div className="panel">
        <h3>Recent AI requests</h3>
        {!ai?.length ? <p className="muted small">None.</p> : (
          <table className="table"><thead><tr><th>When</th><th>Model</th><th>Purpose</th><th>Sent</th><th>Items</th><th>Result</th></tr></thead>
            <tbody>{ai.slice(0, 20).map((r, i) => <tr key={i}><td>{fmtWhen(r.at)}</td><td>{r.model}</td><td>{r.purpose}</td><td>{r.request_chars} chars</td><td>{r.items}{r.rejected ? ` (+${r.rejected} rejected)` : ""}</td><td>{r.ok ? `${r.ms} ms` : r.error}</td></tr>)}</tbody>
          </table>
        )}
      </div>
      <div className="panel">
        <h3>Audit log</h3>
        <div className="hist">{log?.map((e, i) => <div key={i}><time>{fmtWhen(e.at)}</time><span>{e.actor} · {e.entity} {e.action}{e.to ? ` → ${e.to}` : ""}{e.reason ? ` — ${e.reason}` : ""}</span></div>)}</div>
      </div>
    </>
  );
}
