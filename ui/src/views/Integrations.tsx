import { useEffect, useState } from "react";
import { AlertTriangle, FileUp, KeyRound, RefreshCw, Unplug } from "lucide-react";
import { Account, api, Integrations as I, onConnect, Prefs, Provider } from "../api";
import { Modal, Toggle, fmtWhen, useAct, useData, useToast } from "../components/ui";
import { ConnectionWizard, SettingsFilePanel } from "./Connections";

const CATS: [string, string][] = [["email", "Email and calendar"], ["chat", "Workplace chat"], ["tasks", "Project and task tools"], ["documents", "Documents and files"]];
const STATUS: Record<string, [string, string]> = {
  connected: ["Connected", "pine"], syncing: ["Syncing", "pine"], attention: ["Needs attention", "amber"], error: ["Error", "brick"], connecting: ["Connecting", ""], disconnected: ["Disconnected", ""],
};
const ONLINE = ["google", "microsoft", "gdrive", "onedrive", "teams", "slack", "jira", "asana", "todoist", "linear"];
/** Which app registration a browser sign-in uses. */
const APP_FOR: Record<string, "google" | "microsoft" | "slack"> = { google: "google", gdrive: "google", microsoft: "microsoft", onedrive: "microsoft", teams: "microsoft", slack: "slack" };
const TOKEN_HELP: Record<string, { where: string; url: string; revoke: string }> = {
  jira: { where: "id.atlassian.com → Security → Create and manage API tokens → Create API token", url: "https://id.atlassian.com/manage-profile/security/api-tokens", revoke: "id.atlassian.com → Security → API tokens" },
  asana: { where: "Asana → Settings → Apps → Developer apps → Create new token", url: "https://app.asana.com/0/my-apps", revoke: "Asana → Settings → Apps → Developer apps" },
  todoist: { where: "Todoist → Settings → Integrations → Developer → copy API token", url: "https://app.todoist.com/app/settings/integrations/developer", revoke: "Todoist → Settings → Integrations → Developer → Issue a new API token" },
  linear: { where: "Linear → Settings → Security & access → Personal API keys → New key", url: "https://linear.app/settings/account/security", revoke: "Linear → Settings → Security & access" },
};

export function Integrations() {
  const [d, reload] = useData<I>(api.integrations);
  const [prefs, reloadPrefs] = useData<Prefs>(api.prefs);
  const [setup, setSetup] = useState<"google" | "microsoft" | "slack">();
  const [tokenFor, setTokenFor] = useState<Provider>();
  const [connecting, setConnecting] = useState<string>();
  const [disc, setDisc] = useState<Account>();
  const act = useAct();
  const toast = useToast();

  useEffect(() => {
    const un = onConnect((p) => {
      setConnecting(undefined);
      if (p.ok) toast(`Connected ${p.account}. First sync is starting.`);
      else toast(`Couldn't connect: ${p.error}`, true);
      reload();
    });
    return () => { un.then((f) => f()); };
  }, [reload, toast]);

  if (!d || !prefs) return <div className="page"><p className="muted">Loading…</p></div>;
  const sample = d.accounts.find((a) => a.provider === "sample");
  const online = d.accounts.filter((a) => ONLINE.includes(a.provider));
  const configured = (p: string) => {
    const app = APP_FOR[p];
    return app === "google" ? d.google_configured : app === "microsoft" ? d.microsoft_configured : app === "slack" ? d.slack_configured : true;
  };

  const connect = (p: Provider) => {
    if (p.support === "token") return setTokenFor(p);
    if (!configured(p.id)) return setSetup(APP_FOR[p.id]);
    setConnecting(p.id);
    act(() => api.connect(p.id), "Finish signing in in your browser. Kept is waiting for the sign-in to complete.");
  };

  return (
    <div className="page">
      <div className="page-head">
        <div><h1>Integrations</h1><div className="sub">Choose what Kept may read. Kept never sends, posts, accepts, deletes or edits anything — the one exception is “Send to Todoist”, which always asks you first.</div></div>
        <button className="btn" onClick={() => act(() => api.syncNow(), (m) => m)}><RefreshCw size={14} /> Sync all now</button>
      </div>
      {!d.keychain_ok && <div className="banner bad"><AlertTriangle size={16} /> Windows Credential Manager isn't available, so sign-ins can't be saved securely and will be forgotten when Kept quits.</div>}

      {online.length > 0 && (
        <div className="section">
          <div className="section-head"><h2>Connected accounts</h2></div>
          <div className="list">{online.map((a) => <AccountRow key={a.id} a={a} onDisconnect={() => setDisc(a)} reload={reload} />)}</div>
        </div>
      )}

      <div className="section">
        <div className="section-head"><h2>Your connections</h2></div>
        <p className="small muted" style={{ margin: "-4px 0 10px", maxWidth: "75ch" }}>
          Google, Microsoft and Slack connect through an app registration that <b>you</b> own (or your IT team does), so Kept never depends on anyone else. Kept walks you through it the first time you click Connect. Jira, Asana, Todoist and Linear just need a personal token.
        </p>
        <div className="list"><SettingsFilePanel onImported={() => { reload(); reloadPrefs(); }} /></div>
      </div>

      {CATS.map(([cat, label]) => {
        const ps = d.providers.filter((p) => p.category === cat);
        if (!ps.length) return null;
        return (
          <div className="section" key={cat}>
            <div className="section-head"><h2>{label}</h2></div>
            <div className="list">
              {ps.map((p) => {
                const connected = online.some((a) => a.provider === p.id);
                return (
                  <div className="prov" key={p.id}>
                    <div>
                      <strong>{p.name}</strong> {connected && <span className="pill pine">Connected</span>}
                      <p>{p.description}</p>
                      <ul className="perm">{p.permissions.map((x) => <li key={x}>{x}</li>)}</ul>
                      {p.support === "oauth" && !configured(p.id) && <p className="small muted">First time: Kept guides you through a one-time setup ({APP_FOR[p.id] === "google" ? "about 10 minutes" : APP_FOR[p.id] === "microsoft" ? "about 5 minutes" : "about 3 minutes"}), or import a settings file from IT.</p>}
                    </div>
                    <div className="btn-row" style={{ alignSelf: "start" }}>
                      <button className="btn primary" disabled={!!connecting} onClick={() => connect(p)}>
                        {connecting === p.id ? "Waiting for browser…" : connected ? "Add another" : "Connect"}
                      </button>
                      {p.support === "oauth" && <button className="btn ghost sm" onClick={() => setSetup(APP_FOR[p.id])}><KeyRound size={13} /> {configured(p.id) ? "Change app" : "Set up"}</button>}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        );
      })}

      <div className="section">
        <div className="section-head"><h2>Files and sample data</h2></div>
        <div className="list">
          <div className="prov">
            <div><strong>Import files</strong><p>Saved emails (.eml), calendar exports (.ics) or a Kept JSON bundle. Only the files you pick are read.</p></div>
            <div><button className="btn" onClick={() => act(() => api.importFiles(), (m) => m ?? undefined).then(reload)}><FileUp size={14} /> Choose files</button></div>
          </div>
          <div className="prov">
            <div><strong>Sample workspace</strong><p>Realistic, made-up emails, chats and meetings, to see what Kept detects before connecting anything.</p></div>
            <div className="btn-row">
              {sample ? <button className="btn danger" onClick={() => act(() => api.disconnect(sample.id, true), "Sample workspace removed").then(reload)}>Remove sample data</button>
                : <button className="btn" onClick={() => act(() => api.loadSample(), (m) => m).then(reload)}>Load sample data</button>}
            </div>
          </div>
        </div>
      </div>

      {setup && <ConnectionWizard kind={setup} prefs={prefs} onClose={() => setSetup(undefined)} onSaved={() => { reload(); reloadPrefs(); }} />}
      {tokenFor && <TokenModal p={tokenFor} onClose={() => { setTokenFor(undefined); reload(); }} />}
      {disc && <DisconnectModal a={disc} onClose={() => { setDisc(undefined); reload(); }} />}
    </div>
  );
}

function AccountRow({ a, onDisconnect, reload }: { a: Account; onDisconnect: () => void; reload: () => void }) {
  const act = useAct();
  const [label, tone] = STATUS[a.status] ?? [a.status, ""];
  const stale = a.last_success_at && Date.now() - new Date(a.last_success_at).getTime() > 6 * 3600 * 1000;
  const setStream = (s: string, on: boolean) => act(() => api.setStreams(a.id, on ? [...a.enabled_streams, s] : a.enabled_streams.filter((x) => x !== s)), "Saved").then(reload);
  const unit = a.category === "tasks" ? "tasks" : a.category === "documents" ? "documents" : "items";
  const isToken = ["jira", "asana", "todoist", "linear"].includes(a.provider);
  return (
    <div className="prov">
      <div>
        <strong>{a.display_name}</strong> <span className={`pill ${tone}`}>{label}</span> {stale && a.status === "connected" && <span className="pill amber">Stale</span>}
        <p className="small">Last successful sync {fmtWhen(a.last_success_at)} · {a.items_synced} {unit} · next {a.backoff_until ? `retry ${fmtWhen(a.backoff_until)}` : fmtWhen(a.next_sync_at)}</p>
        {a.status_detail && <p className="small" style={{ color: a.status === "connected" ? "var(--ink-3)" : "var(--brick)" }}>{a.status_detail}</p>}
        {(a.provider === "google" || a.provider === "microsoft") && (
          <div className="btn-row" style={{ marginTop: 6 }}>
            <Toggle checked={a.enabled_streams.includes("mail")} onChange={(v) => setStream("mail", v)} label="Mail" />
            <Toggle checked={a.enabled_streams.includes("calendar")} onChange={(v) => setStream("calendar", v)} label="Calendar" />
          </div>
        )}
      </div>
      <div className="btn-row" style={{ alignSelf: "start" }}>
        {a.status === "attention" ? (
          isToken ? <span className="small" style={{ color: "var(--brick)" }}>Token rejected — disconnect and connect with a new token.</span>
            : <button className="btn primary" onClick={() => act(() => api.connect(a.provider), "Sign in again in your browser.")}>Reconnect</button>
        ) : <button className="btn" onClick={() => act(() => api.syncNow(a.id), (m) => m).then(reload)}><RefreshCw size={14} /> Sync now</button>}
        <button className="btn ghost" onClick={onDisconnect}><Unplug size={14} /> Disconnect</button>
      </div>
    </div>
  );
}

function TokenModal({ p, onClose }: { p: Provider; onClose: () => void }) {
  const [token, setToken] = useState("");
  const [email, setEmail] = useState("");
  const [site, setSite] = useState("");
  const [busy, setBusy] = useState(false);
  const act = useAct();
  const help = TOKEN_HELP[p.id];
  const ready = token.trim().length >= 8 && (p.id !== "jira" || (email.includes("@") && site.trim().length > 3));
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    const r = await act(() => api.connectToken(p.id, token.trim(), email.trim() || undefined, site.trim() || undefined), (m) => m);
    setBusy(false);
    if (r !== undefined) onClose();
  };
  return (
    <Modal onClose={onClose} title={`Connect ${p.name}`}>
      <form onSubmit={submit} style={{ display: "grid", gap: 12 }}>
        <p className="small" style={{ margin: 0 }}>
          Kept uses your personal API token. Create one at <strong>{help.where}</strong>{" "}
          (<a href="#" onClick={(e) => { e.preventDefault(); act(() => api.openExternal(help.url)); }}>open page</a>). Kept checks the token, then stores it only in Windows Credential Manager. You can revoke it any time in {help.revoke}.
        </p>
        {p.id === "jira" && (
          <>
            <label className="field">Jira site<input className="input" value={site} onChange={(e) => setSite(e.target.value)} placeholder="your-company.atlassian.net" /></label>
            <label className="field">Your Atlassian account email<input className="input" type="email" value={email} onChange={(e) => setEmail(e.target.value)} /></label>
          </>
        )}
        <label className="field">{p.id === "linear" ? "Personal API key" : "API token"}<input className="input" type="password" autoComplete="off" autoFocus value={token} onChange={(e) => setToken(e.target.value)} /></label>
        <p className="small muted" style={{ margin: 0 }}>The token acts as you. Kept only reads tasks assigned to you{p.id === "todoist" ? ", and creates a Todoist task only when you click “Send to Todoist” and confirm" : ""}.</p>
        <div className="btn-row"><button className="btn primary" type="submit" disabled={!ready || busy}>{busy ? "Checking…" : "Connect"}</button><button className="btn ghost" type="button" onClick={onClose}>Cancel</button></div>
      </form>
    </Modal>
  );
}

function DisconnectModal({ a, onClose }: { a: Account; onClose: () => void }) {
  const [del, setDel] = useState(true);
  const act = useAct();
  const tool = ["jira", "asana", "todoist", "linear"].includes(a.provider);
  return (
    <Modal onClose={onClose} title={`Disconnect ${a.display_name}?`}>
      <p>
        Kept will {["google", "gdrive", "slack"].includes(a.provider) ? "revoke its access token, " : ""}delete the saved sign-in from Windows Credential Manager and remove everything it synced from this account.
      </p>
      {["microsoft", "onedrive", "teams"].includes(a.provider) && <p className="small muted">Microsoft doesn't let desktop apps revoke their own sign-in. To remove Kept's permission completely, also visit myapps.microsoft.com → Kept → Revoke.</p>}
      {tool && <p className="small muted">Your API token stays valid until you revoke it in {TOKEN_HELP[a.provider].revoke}.</p>}
      <Toggle checked={del} onChange={setDel} label={tool ? "Also remove the tasks Kept copied from this tool" : "Also delete unreviewed suggestions from this account"} desc="Tasks you created or accepted from email and chat are kept." />
      <div className="btn-row" style={{ marginTop: 14 }}>
        <button className="btn danger" onClick={() => act(() => api.disconnect(a.id, del), "Disconnected and local data removed").then(onClose)}>Disconnect</button>
        <button className="btn ghost" onClick={onClose}>Cancel</button>
      </div>
    </Modal>
  );
}
