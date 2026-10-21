// Guided "use your own app" setup for Google, Microsoft and Slack, plus settings-file import/export.
// Everything here is owned by the user (or their IT team) — Kept never depends on its publisher.
import { useEffect, useState } from "react";
import { Check, Copy, ExternalLink, FileDown, FileUp } from "lucide-react";
import { api, Prefs } from "../api";
import { Modal, Toggle, useAct } from "../components/ui";
import { invoke } from "@tauri-apps/api/core";

export type AppKind = "google" | "microsoft" | "slack";

const SLACK_MANIFEST = {
  display_information: { name: "Kept", description: "Personal assistant that tracks your commitments (read-only)" },
  oauth_config: {
    redirect_urls: ["http://localhost:53682/callback"],
    scopes: { user: ["im:history", "im:read", "mpim:history", "mpim:read", "channels:history", "groups:history", "users:read", "users:read.email", "search:read"] },
    pkce_enabled: true,
    token_rotation_enabled: true,
  },
  settings: { org_deploy_enabled: false, socket_mode_enabled: false },
};
const SLACK_CREATE_URL = `https://api.slack.com/apps?new_app=1&manifest_json=${encodeURIComponent(JSON.stringify(SLACK_MANIFEST))}`;
const MS_PERMS = "Mail.Read, Calendars.Read, Chat.Read, Files.Read.All, User.Read, offline_access";

type Step = { title: string; body: React.ReactNode; open?: [string, string][]; copy?: [string, string][] };

function steps(kind: AppKind): Step[] {
  if (kind === "google")
    return [
      { title: "Create a Google Cloud project", body: <>Sign in with the Google account you'll connect. Name the project <b>Kept</b> and click Create.</>, open: [["Open Google Cloud", "https://console.cloud.google.com/projectcreate"]] },
      { title: "Turn on Gmail, Calendar and Drive", body: <>Check the Kept project is selected at the top, then click <b>Enable</b>.</>, open: [["Enable the APIs", "https://console.cloud.google.com/flows/enableapi?apiid=gmail.googleapis.com,calendar-json.googleapis.com,drive.googleapis.com"]] },
      { title: "Set up the sign-in screen", body: <>Click <b>Get started</b>. App name: <b>Kept</b>, your email as support and contact. For <b>Audience</b> choose <b>External</b> — or <b>Internal</b> if your company uses Google Workspace and only colleagues will use it (then skip step 4).</>, open: [["Open sign-in screen setup", "https://console.cloud.google.com/auth/branding"]] },
      { title: "Publish it for yourself", body: <>Click <b>Publish app</b> and confirm. The app belongs only to you, so Google doesn't review it. Publishing stops Google signing you out every 7 days.</>, open: [["Open Audience", "https://console.cloud.google.com/auth/audience"]] },
      { title: "Create the desktop sign-in key", body: <>Application type <b>Desktop app</b>, name <b>Kept</b>, then <b>Create</b>. Keep the window with the <b>Client ID</b> and <b>Client secret</b> open.</>, open: [["Create the key", "https://console.cloud.google.com/auth/clients/create"]] },
    ];
  if (kind === "microsoft")
    return [
      { title: "Register an app", body: <>Click <b>New registration</b>. Name: <b>Kept</b>. Who can use it: <b>Accounts in any organizational directory and personal Microsoft accounts</b> (or your organisation only). Redirect URI: platform <b>Public client/native (mobile &amp; desktop)</b>, value below. Then <b>Register</b>.</>, open: [["Work or school account", "https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade"], ["Personal Outlook.com", "https://portal.azure.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade"]], copy: [["Redirect URI", "http://localhost"]] },
      { title: "Allow Kept to read", body: <>In the new app: <b>API permissions → Add a permission → Microsoft Graph → Delegated permissions</b>, then tick each of these. They're all read-only.</>, copy: [["Permissions", MS_PERMS]] },
      { title: "Copy the Application (client) ID", body: <>It's on the app's <b>Overview</b> page. If your company doesn't let you register apps, ask IT — they can set it up once for everyone and send you a settings file.</> },
    ];
  return [
    { title: "Create the Kept app in Slack", body: <>This opens Slack with everything filled in. Pick your workspace, review, and click <b>Create</b>.</>, open: [["Create the Slack app", SLACK_CREATE_URL]] },
    { title: "Check PKCE is on", body: <>In <b>OAuth &amp; Permissions</b>, make sure <b>PKCE</b> is enabled (it lets Kept sign in without a secret). The redirect URL should be the value below.</>, copy: [["Redirect URL", "http://localhost:53682/callback"]] },
    { title: "Copy the Client ID", body: <>In <b>Basic Information → App Credentials</b>. You don't need the secret. If your workspace requires approval, Slack will ask an admin when you connect.</> },
  ];
}

function CopyButton({ label, value }: { label: string; value: string }) {
  const [done, setDone] = useState(false);
  return (
    <button type="button" className="btn sm" onClick={() => { navigator.clipboard?.writeText(value).then(() => { setDone(true); setTimeout(() => setDone(false), 1500); }); }} title={value}>
      {done ? <Check size={13} /> : <Copy size={13} />} {done ? "Copied" : `Copy ${label.toLowerCase()}`}
    </button>
  );
}

function Field({ label, kind, value, onChange, secret, hint }: { label: string; kind: string; value: string; onChange: (v: string) => void; secret?: boolean; hint?: string }) {
  const [problem, setProblem] = useState<string | null>(null);
  useEffect(() => {
    const t = setTimeout(() => { invoke<string | null>("check_connection_value", { kind, value }).then(setProblem).catch(() => setProblem(null)); }, 250);
    return () => clearTimeout(t);
  }, [kind, value]);
  return (
    <label className="field">
      {label} {hint && <span className="hint">{hint}</span>}
      <input className="input" type={secret ? "password" : "text"} autoComplete="off" value={value} onChange={(e) => onChange(e.target.value)} style={problem ? { borderColor: "var(--brick)" } : undefined} />
      {problem && <span className="small" style={{ color: "var(--brick)", fontWeight: 400 }}>{problem}</span>}
    </label>
  );
}

const TITLES: Record<AppKind, string> = { google: "Connect Google with your own app", microsoft: "Connect Microsoft with your own app", slack: "Connect Slack with your own app" };
const INTRO: Record<AppKind, string> = {
  google: "Google asks every app that reads Gmail to be registered. You'll register Kept under your own Google account — about 10 minutes, once. Only you can use it, and nothing goes through anyone else.",
  microsoft: "Microsoft asks every app that reads mail to be registered. You'll register Kept in your own Microsoft account — about 5 minutes, once. It covers Outlook, Calendar, Teams and OneDrive.",
  slack: "Slack asks every app that reads messages to be a Slack app in your workspace. Kept fills in the details for you — about 3 minutes, once.",
};

export function ConnectionWizard({ kind, prefs, onClose, onSaved }: { kind: AppKind; prefs: Prefs; onClose: () => void; onSaved?: () => void }) {
  const act = useAct();
  const [id, setId] = useState(kind === "google" ? prefs.google_client_id : kind === "microsoft" ? prefs.microsoft_client_id : prefs.slack_client_id);
  const [secret, setSecret] = useState("");
  const [tenant, setTenant] = useState(prefs.microsoft_tenant || "common");
  const [saving, setSaving] = useState(false);
  const list = steps(kind);
  const save = async () => {
    setSaving(true);
    const r = await act(() => invoke("save_connection", { provider: kind, clientId: id, clientSecret: secret || null, tenant }), "Saved. Now click Connect on the service you want.");
    setSaving(false);
    if (r !== undefined) { onSaved?.(); onClose(); }
  };
  return (
    <Modal onClose={onClose} title={TITLES[kind]}>
      <p className="small" style={{ marginTop: 0 }}>{INTRO[kind]}</p>
      <ol style={{ paddingLeft: 20, margin: "0 0 14px", display: "grid", gap: 12 }}>
        {list.map((s, i) => (
          <li key={i}>
            <strong>{s.title}</strong>
            <div className="small" style={{ margin: "3px 0 6px" }}>{s.body}</div>
            <div className="btn-row">
              {s.open?.map(([l, u]) => <button key={u} type="button" className="btn sm" onClick={() => act(() => api.openExternal(u))}><ExternalLink size={13} /> {l}</button>)}
              {s.copy?.map(([l, v]) => <CopyButton key={l} label={l} value={v} />)}
            </div>
          </li>
        ))}
        <li>
          <strong>Paste it here</strong>
          <div style={{ display: "grid", gap: 10, marginTop: 6 }}>
            {kind === "google" && (
              <>
                <Field label="Client ID" kind="google_client_id" value={id} onChange={setId} />
                <Field label="Client secret" kind="google_client_secret" value={secret} onChange={setSecret} secret hint="Saved in Windows Credential Manager. Leave blank to keep the saved one." />
              </>
            )}
            {kind === "microsoft" && (
              <>
                <Field label="Application (client) ID" kind="microsoft_client_id" value={id} onChange={setId} />
                <Field label="Tenant" kind="microsoft_tenant" value={tenant} onChange={setTenant} hint="Leave as “common” unless IT gave you a tenant ID or domain." />
              </>
            )}
            {kind === "slack" && <Field label="Client ID" kind="slack_client_id" value={id} onChange={setId} />}
          </div>
        </li>
      </ol>
      {kind === "google" && <p className="small muted">When you connect, Google shows “Google hasn't verified this app”. That's expected — it's your own app. Click <b>Advanced → Go to Kept</b>.</p>}
      <div className="btn-row"><button className="btn primary" disabled={!id.trim() || saving} onClick={save}>Save</button><button className="btn ghost" onClick={onClose}>Cancel</button></div>
    </Modal>
  );
}

/** Import a settings file from IT, or export yours for colleagues. */
export function SettingsFilePanel({ onImported }: { onImported: () => void }) {
  const act = useAct();
  const [exporting, setExporting] = useState(false);
  const [org, setOrg] = useState("");
  const [withSecret, setWithSecret] = useState(true);
  return (
    <>
      <div className="prov">
        <div>
          <strong>Got a settings file from your IT team?</strong>
          <p>Import it and the Google, Microsoft and Slack connections are ready — just click Connect and sign in with your own account.</p>
        </div>
        <div className="btn-row" style={{ alignSelf: "start" }}>
          <button className="btn primary" onClick={() => act(() => invoke<string | null>("import_connection_settings"), (m) => m ?? undefined).then(onImported)}><FileUp size={14} /> Import settings file</button>
          <button className="btn ghost sm" onClick={() => setExporting(true)}><FileDown size={13} /> Share my setup with my team</button>
        </div>
      </div>
      {exporting && (
        <Modal onClose={() => setExporting(false)} title="Share your connection setup">
          <p className="small" style={{ marginTop: 0 }}>Creates a file with the app registrations you set up, so colleagues can import it instead of registering their own. It contains no sign-ins, tokens or data — each person still signs in to their own account.</p>
          <label className="field">Organisation name (shown to people who import it)<input className="input" value={org} onChange={(e) => setOrg(e.target.value)} placeholder="e.g. Acme Ltd" /></label>
          <Toggle checked={withSecret} onChange={setWithSecret} label="Include the Google client secret" desc="Needed for Google sign-in. Google treats desktop-app secrets as non-confidential, but share the file only inside your organisation." />
          <div className="btn-row" style={{ marginTop: 12 }}>
            <button className="btn primary" onClick={() => act(() => invoke<string | null>("export_connection_settings", { includeGoogleSecret: withSecret, organisation: org || null }), (f) => (f ? `Saved to ${f}` : undefined)).then(() => setExporting(false))}>Save file</button>
            <button className="btn ghost" onClick={() => setExporting(false)}>Cancel</button>
          </div>
          {!withSecret && <p className="small muted">Without the secret, colleagues can import Microsoft and Slack settings but must add the Google secret themselves.</p>}
        </Modal>
      )}
    </>
  );
}
