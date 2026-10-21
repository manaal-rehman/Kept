import { useState } from "react";
import { Bell, Eye, KeyRound, Lock, ShieldCheck } from "lucide-react";
import { api, Prefs } from "../api";
import { useAct } from "../components/ui";

export function Onboarding({ prefs, done }: { prefs: Prefs; done: (go?: string) => void }) {
  const [step, setStep] = useState(0);
  const [p, setP] = useState<Prefs>(() => ({ ...prefs, timezone: prefs.timezone === "UTC" ? Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC" : prefs.timezone }));
  const [emails, setEmails] = useState(prefs.user_emails.join(", "));
  const [start, setStart] = useState<"sample" | "connect" | "empty">("sample");
  const [busy, setBusy] = useState(false);
  const act = useAct();

  const finish = async () => {
    setBusy(true);
    const final = { ...p, onboarded: true, user_emails: emails.split(/[,\s]+/).map((x) => x.trim().toLowerCase()).filter((x) => x.includes("@")) };
    await act(() => api.savePrefs(final));
    if (start === "sample") await act(() => api.loadSample(), (m) => m);
    setBusy(false);
    done(start === "connect" ? "integrations" : "home");
  };

  return (
    <div className="ob">
      <div className="ob-card">
        <div className="ob-steps" aria-label={`Step ${step + 1} of 3`}>{[0, 1, 2].map((i) => <i key={i} className={i <= step ? "on" : ""} />)}</div>

        {step === 0 && (
          <>
            <h1>Kept remembers what you promised.</h1>
            <p className="lead">It reads the work accounts you choose, finds the commitments hidden in your conversations — yours and the ones made to you — and reminds you before anything slips. Here's exactly what it does with access:</p>
            <ul className="access">
              <li><Eye size={18} /><div><strong>Reads, never writes.</strong> Kept asks only for read-only access. It can't send email, post messages, accept invites or change your calendar.</div></li>
              <li><Lock size={18} /><div><strong>Stays on this computer.</strong> Tasks and message excerpts are stored in your Windows profile. Sign-ins go to Windows Credential Manager. There's no Kept cloud.</div></li>
              <li><KeyRound size={18} /><div><strong>AI is optional and yours.</strong> If you add an AI key, Kept sends one message at a time to that provider for analysis, with credentials stripped. Without one it uses built-in rules.</div></li>
              <li><ShieldCheck size={18} /><div><strong>You stay in charge.</strong> Anything uncertain goes to Review. Every task shows the exact words it came from, and every change can be undone.</div></li>
              <li><Bell size={18} /><div><strong>Reminds, doesn't nag.</strong> One alert per stage, quiet hours, and an hourly limit you control.</div></li>
            </ul>
            <div className="btn-row"><button className="btn primary" onClick={() => setStep(1)}>Set up Kept</button></div>
          </>
        )}

        {step === 1 && (
          <>
            <h1>About you</h1>
            <p className="lead">Kept uses this to tell your promises apart from everyone else's, and to schedule reminders in your working day.</p>
            <div style={{ display: "grid", gap: 14 }}>
              <label className="field">Your name<input className="input" autoFocus value={p.user_name} onChange={(e) => setP({ ...p, user_name: e.target.value })} placeholder="e.g. Sara Lindqvist" /></label>
              <label className="field">Your work email addresses <span className="hint">Comma separated. Connected accounts are added automatically.</span><input className="input" value={emails} onChange={(e) => setEmails(e.target.value)} placeholder="you@company.com" /></label>
              <div className="grid3">
                <label className="field">Timezone<input className="input" value={p.timezone} onChange={(e) => setP({ ...p, timezone: e.target.value })} /></label>
                <label className="field">Workday starts<input className="input" type="time" value={p.work_start} onChange={(e) => setP({ ...p, work_start: e.target.value })} /></label>
                <label className="field">Workday ends<input className="input" type="time" value={p.work_end} onChange={(e) => setP({ ...p, work_end: e.target.value })} /></label>
              </div>
            </div>
            <div className="btn-row" style={{ marginTop: 20 }}><button className="btn primary" onClick={() => setStep(2)} disabled={!p.user_name.trim()}>Continue</button><button className="btn ghost" onClick={() => setStep(0)}>Back</button></div>
          </>
        )}

        {step === 2 && (
          <>
            <h1>How would you like to start?</h1>
            <p className="lead">You can change this any time in Integrations.</p>
            {([
              ["sample", "Explore with sample data", "A realistic, made-up workspace: see what Kept detects before connecting anything. Remove it with one click later."],
              ["connect", "Connect my accounts", "Opens Integrations. Kept guides you through a one-time setup for Google, Microsoft or Slack, or you can import a settings file from your IT team. Task tools just need a token."],
              ["empty", "Start empty", "Add tasks yourself or import .eml / .ics files."],
            ] as const).map(([k, t, d]) => (
              <div key={k} className="choice" role="radio" aria-checked={start === k} tabIndex={0} onClick={() => setStart(k)} onKeyDown={(e) => e.key === "Enter" && setStart(k)}>
                <input type="radio" checked={start === k} readOnly tabIndex={-1} />
                <div><strong>{t}</strong><div className="small muted">{d}</div></div>
              </div>
            ))}
            <div className="btn-row" style={{ marginTop: 20 }}><button className="btn primary" disabled={busy} onClick={finish}>{busy ? "Setting up…" : "Finish"}</button><button className="btn ghost" onClick={() => setStep(1)}>Back</button></div>
          </>
        )}
      </div>
    </div>
  );
}
