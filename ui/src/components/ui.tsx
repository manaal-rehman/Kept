import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { Check, X } from "lucide-react";
import type { SourceRef, Task } from "../api";

// ---------- formatting ----------
// All times are shown in the timezone chosen in Settings (defaults to the system one).
let TZ: string | undefined;
export function setDisplayTz(tz?: string) {
  try {
    if (tz) new Intl.DateTimeFormat(undefined, { timeZone: tz });
    TZ = tz || undefined;
  } catch {
    TZ = undefined;
  }
}
const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

export function fmtDue(iso?: string | null, tz: string | undefined = TZ): string {
  if (!iso) return "";
  const d = new Date(iso);
  const now = new Date();
  const opts: Intl.DateTimeFormatOptions = { timeZone: tz };
  const day = (x: Date) => x.toLocaleDateString("en-CA", opts);
  const time = d.toLocaleTimeString(undefined, { ...opts, hour: "2-digit", minute: "2-digit" });
  const diffDays = Math.round((new Date(day(d)).getTime() - new Date(day(now)).getTime()) / 86400000);
  if (diffDays === 0) return `Today ${time}`;
  if (diffDays === 1) return `Tomorrow ${time}`;
  if (diffDays === -1) return `Yesterday ${time}`;
  if (diffDays > 1 && diffDays < 7) return d.toLocaleDateString(undefined, { ...opts, weekday: "short" }) + ` ${time}`;
  return d.toLocaleDateString(undefined, { ...opts, day: "numeric", month: "short" }) + (diffDays < 0 ? "" : ` ${time}`);
}

export function fmtWhen(iso?: string | null): string {
  if (!iso) return "never";
  const s = (new Date(iso).getTime() - Date.now()) / 1000;
  const a = Math.abs(s);
  if (a < 60) return "just now";
  if (a < 3600) return rtf.format(Math.round(s / 60), "minute");
  if (a < 86400) return rtf.format(Math.round(s / 3600), "hour");
  return rtf.format(Math.round(s / 86400), "day");
}

export function fmtTime(iso: string, tz: string | undefined = TZ) {
  return new Date(iso).toLocaleTimeString(undefined, { timeZone: tz, hour: "2-digit", minute: "2-digit" });
}

export function dueClass(t: Task): string {
  if (!t.due_at) return "due";
  const ms = new Date(t.due_at).getTime() - Date.now();
  const closed = ["completed", "cancelled", "rejected"].includes(t.status);
  let c = "due";
  if (!closed && ms < 0) c += " late";
  else if (!closed && ms < 36 * 3600 * 1000) c += " soon";
  if (t.due_kind === "inferred") c += " inferred";
  return c;
}

export const STATUS_LABEL: Record<string, string> = {
  detected: "Detected", needs_confirmation: "Needs review", ready: "To do", in_progress: "In progress", waiting: "Waiting on someone",
  blocked: "Blocked", draft_prepared: "Draft prepared", sent: "Sent", awaiting_approval: "Awaiting approval", completed: "Done",
  cancelled: "Cancelled", rejected: "Rejected",
};
export const TYPE_LABEL: Record<string, string> = {
  action: "Action", reply: "Reply", approval: "Approval", deliverable: "Deliverable", followup: "Follow-up", prep: "Meeting prep", review: "Review",
};
export const DUE_KIND_LABEL: Record<string, string> = {
  explicit: "stated in the source", inferred: "inferred from wording — please check", suggested: "suggestion, needs your confirmation", confirmed: "confirmed by you", none: "",
};
export const PRIORITY_LABEL = ["", "Critical", "High", "Normal", "Low"];

export function confLevel(c: number) {
  return c >= 0.85 ? 3 : c >= 0.65 ? 2 : 1;
}

export function Confidence({ value }: { value: number }) {
  const l = confLevel(value);
  const words = ["", "Low confidence", "Medium confidence", "High confidence"];
  return (
    <span className={`conf c${l}`} title={`${words[l]} (${Math.round(value * 100)}%)`} aria-label={words[l]}>
      <i /><i /><i />
    </span>
  );
}

export function SourceNote({ s, compact }: { s?: SourceRef | null; compact?: boolean }) {
  if (!s?.evidence_quote) return null;
  const rel: Record<string, string> = { origin: "", update: "Update", followup: "Follow-up", evidence: "Evidence", completion: "Completion", duplicate: "Duplicate" };
  return (
    <div className={`note ${s.relation === "followup" ? "warn" : ""}`}>
      <q>{compact && s.evidence_quote.length > 160 ? s.evidence_quote.slice(0, 157) + "…" : s.evidence_quote}</q>
      <span className="from">
        {rel[s.relation] ? `${rel[s.relation]} — ` : ""}
        {s.label}
      </span>
    </div>
  );
}

export function Toggle({ checked, onChange, label, desc, disabled }: { checked: boolean; onChange: (v: boolean) => void; label: string; desc?: string; disabled?: boolean }) {
  return (
    <label className="toggle">
      <input type="checkbox" role="switch" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span>
        <span className="t-label">{label}</span>
        {desc && <span className="t-desc">{desc}</span>}
      </span>
    </label>
  );
}

export function Empty({ title, children }: { title: string; children?: React.ReactNode }) {
  return (
    <div className="empty">
      <strong>{title}</strong>
      {children}
    </div>
  );
}

export function Drawer({ onClose, children, head }: { onClose: () => void; children: React.ReactNode; head: React.ReactNode }) {
  useEscape(onClose);
  return (
    <>
      <div className="scrim" onClick={onClose} />
      <aside className="drawer" role="dialog" aria-modal="true">
        <div className="drawer-head">
          <div style={{ flex: 1, minWidth: 0 }}>{head}</div>
          <button className="btn ghost sm" onClick={onClose} aria-label="Close"><X size={16} /></button>
        </div>
        <div className="drawer-body">{children}</div>
      </aside>
    </>
  );
}

export function Modal({ onClose, children, title }: { onClose: () => void; children: React.ReactNode; title: string }) {
  useEscape(onClose);
  return (
    <>
      <div className="scrim" onClick={onClose} />
      <div className="modal" role="dialog" aria-modal="true" aria-label={title}>
        <h2 style={{ fontSize: 18 }}>{title}</h2>
        {children}
      </div>
    </>
  );
}

function useEscape(cb: () => void) {
  const ref = useRef(cb);
  ref.current = cb;
  useEffect(() => {
    const h = (e: KeyboardEvent) => e.key === "Escape" && ref.current();
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, []);
}

// ---------- toasts ----------
type T = { id: number; text: string; bad?: boolean };
const ToastCtx = createContext<(text: string, bad?: boolean) => void>(() => {});
export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [items, setItems] = useState<T[]>([]);
  const push = useCallback((text: string, bad?: boolean) => {
    const id = Date.now() + Math.random();
    setItems((x) => [...x.slice(-2), { id, text, bad }]);
    setTimeout(() => setItems((x) => x.filter((i) => i.id !== id)), bad ? 7000 : 4200);
  }, []);
  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div className="toast-stack" role="status" aria-live="polite">
        {items.map((i) => (
          <div key={i.id} className={`toast ${i.bad ? "bad" : ""}`}>{i.text}</div>
        ))}
      </div>
    </ToastCtx.Provider>
  );
}
export const useToast = () => useContext(ToastCtx);

/** Run an async action, toast the error if it fails. */
export function useAct() {
  const toast = useToast();
  return useCallback(
    async <R,>(fn: () => Promise<R>, ok?: string | ((r: R) => string | undefined)): Promise<R | undefined> => {
      try {
        const r = await fn();
        const msg = typeof ok === "function" ? ok(r) : ok;
        if (msg) toast(msg);
        return r;
      } catch (e) {
        toast(String(e instanceof Error ? e.message : e), true);
        return undefined;
      }
    },
    [toast]
  );
}

export function CheckButton({ done, onClick }: { done: boolean; onClick: () => void }) {
  return (
    <button className={`check ${done ? "done" : ""}`} onClick={(e) => { e.stopPropagation(); onClick(); }} aria-label={done ? "Mark as not done" : "Mark as done"} title={done ? "Mark as not done" : "Mark as done"}>
      {done && <Check size={12} strokeWidth={3} />}
    </button>
  );
}

/** Data loader that re-runs whenever the agent reports a change. */
export function useData<T>(load: () => Promise<T>, deps: unknown[] = []): [T | undefined, () => void, string | undefined] {
  const [data, setData] = useState<T>();
  const [err, setErr] = useState<string>();
  const ref = useRef(load);
  ref.current = load;
  const run = useCallback(() => {
    ref.current().then((d) => { setData(d); setErr(undefined); }).catch((e) => setErr(String(e?.message ?? e)));
  }, []);
  useEffect(run, deps); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    const h = () => run();
    window.addEventListener("kept-changed", h);
    return () => window.removeEventListener("kept-changed", h);
  }, [run]);
  return [data, run, err];
}

export function fmtDay(iso: string, tz: string | undefined = TZ) {
  return new Date(iso).toLocaleDateString(undefined, { timeZone: tz, weekday: "long", day: "numeric", month: "long" });
}

export function localInputValue(iso?: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
