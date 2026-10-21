import { useCallback, useEffect, useRef, useState } from "react";
import { Bell, CalendarDays, CheckSquare, Home as HomeIcon, Inbox, Plug, Repeat2, RefreshCw, Search, Settings as Cog } from "lucide-react";
import { api, Dashboard, inTauri, onChanged, Prefs } from "./api";
import { NewTaskModal, TaskDrawer } from "./components/tasks";
import { ToastProvider, setDisplayTz, useAct, useData } from "./components/ui";
import { Home } from "./views/Home";
import { Review } from "./views/Review";
import { Tasks } from "./views/Tasks";
import { Meetings } from "./views/Meetings";
import { Followups } from "./views/Followups";
import { Notifications } from "./views/Notifications";
import { Integrations } from "./views/Integrations";
import { Settings } from "./views/Settings";
import { Onboarding } from "./views/Onboarding";

type View = "home" | "review" | "tasks" | "meetings" | "followups" | "notifications" | "integrations" | "settings";

export default function App() {
  return (
    <ToastProvider>
      <Shell />
    </ToastProvider>
  );
}

function Shell() {
  const [prefs, setPrefs] = useState<Prefs>();
  const [view, setView] = useState<View>("home");
  const [arg, setArg] = useState<string>();
  const [taskId, setTaskId] = useState<string>();
  const [search, setSearch] = useState("");
  const [adding, setAdding] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const act = useAct();
  const [dash] = useData<Dashboard>(api.dashboard);

  const loadPrefs = useCallback(() => api.prefs().then((p) => { setPrefs(p); setDisplayTz(p.timezone); }).catch(() => {}), []);
  useEffect(() => { loadPrefs(); }, [loadPrefs]);
  useEffect(() => {
    // Fan agent "data changed" events out to every view.
    const un = onChanged(() => { window.dispatchEvent(new Event("kept-changed")); loadPrefs(); });
    return () => { un.then((f) => f()); };
  }, [loadPrefs]);

  const go = useCallback((v: string, a?: string) => { setView(v as View); setArg(a); }, []);

  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      const typing = ["INPUT", "TEXTAREA", "SELECT"].includes((e.target as HTMLElement)?.tagName);
      if (typing || e.ctrlKey || e.metaKey) {
        if (e.ctrlKey && e.key.toLowerCase() === "k") { e.preventDefault(); searchRef.current?.focus(); }
        return;
      }
      if (e.key === "/") { e.preventDefault(); searchRef.current?.focus(); }
      else if (e.key.toLowerCase() === "n") { e.preventDefault(); setAdding(true); }
      else if (e.altKey && /^[1-8]$/.test(e.key)) {
        const order: View[] = ["home", "review", "tasks", "meetings", "followups", "notifications", "integrations", "settings"];
        go(order[Number(e.key) - 1]);
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [go]);

  if (!inTauri) {
    return <div className="ob"><div className="ob-card"><h1>Kept runs as a desktop app</h1><p className="lead">This page is the interface bundled inside the Kept application. Start it with <code>npm run app:dev</code> during development, or install the Windows build.</p></div></div>;
  }
  if (!prefs) return null;
  if (!prefs.onboarded) return <Onboarding prefs={prefs} done={(v) => { loadPrefs(); window.dispatchEvent(new Event("kept-changed")); if (v) go(v); }} />;

  const nav: [View, string, any, number?][] = [
    ["home", "Home", HomeIcon],
    ["review", "Review", Inbox, dash?.review_count],
    ["tasks", "My tasks", CheckSquare],
    ["meetings", "Meetings", CalendarDays, dash?.meetings_needing_prep],
    ["followups", "Follow-ups", Repeat2, dash?.followups_count],
    ["notifications", "Notifications", Bell, dash?.unread_notifications],
    ["integrations", "Integrations", Plug],
    ["settings", "Settings", Cog],
  ];
  const sync = async () => { setSyncing(true); await act(() => api.syncNow(), (m) => m); setSyncing(false); };
  const onSearch = (s: string) => { setSearch(s); if (s && view !== "tasks") setView("tasks"); };

  return (
    <div className="shell">
      <nav className="rail" aria-label="Main">
        <div className="brand"><span className="brand-mark" aria-hidden /><span>Kept</span></div>
        {nav.map(([k, label, Icon, count], i) => (
          <button key={k} className="nav" aria-current={view === k ? "page" : undefined} onClick={() => go(k)} title={`${label} (Alt+${i + 1})`}>
            <Icon size={17} /><span className="lbl">{label}</span>
            {!!count && <span className={`count ${k === "review" || k === "meetings" ? "hot" : ""}`}>{count}</span>}
          </button>
        ))}
        <div className="rail-gap" />
        <div className="rail-status">
          <span><span className={`dot ${prefs.monitoring_paused ? "off" : dash?.attention_accounts.length ? "warn" : ""}`} />{prefs.monitoring_paused ? "Monitoring paused" : dash?.attention_accounts.length ? "An account needs attention" : "Watching for new commitments"}</span>
          <span>Closing this window keeps Kept running in the tray.</span>
        </div>
      </nav>
      <main className="main">
        <div className="topbar">
          <label className="search">
            <Search size={15} />
            <input ref={searchRef} value={search} onChange={(e) => onSearch(e.target.value)} placeholder="Search tasks, people, projects" aria-label="Search" />
            <span className="kbd">/</span>
          </label>
          <div style={{ flex: 1 }} />
          <button className="btn ghost" onClick={() => setAdding(true)} title="New task (N)">New task</button>
          <button className="btn" onClick={sync} disabled={syncing}><RefreshCw size={14} className={syncing ? "spin" : ""} /> {syncing ? "Syncing…" : "Sync now"}</button>
        </div>
        {view === "home" && <Home open={setTaskId} go={go} />}
        {view === "review" && <Review open={setTaskId} />}
        {view === "tasks" && <Tasks open={setTaskId} search={search} />}
        {view === "meetings" && <Meetings open={setTaskId} initial={arg} />}
        {view === "followups" && <Followups open={setTaskId} />}
        {view === "notifications" && <Notifications open={setTaskId} go={go} />}
        {view === "integrations" && <Integrations />}
        {view === "settings" && <Settings />}
      </main>
      {taskId && <TaskDrawer id={taskId} onClose={() => setTaskId(undefined)} onOpen={setTaskId} />}
      {adding && <NewTaskModal onClose={() => setAdding(false)} />}
    </div>
  );
}
