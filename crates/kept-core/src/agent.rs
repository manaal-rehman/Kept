//! The background agent: owns the database, runs sync → extract → plan → notify.
//! The desktop shell calls `tick()` every minute from a background task, so all of
//! this keeps working while the dashboard window is closed.

use crate::ai::{self, AiConfig, AiError};
use crate::connectors::{self, docs, files, google, microsoft, oauth, sample, slack, teams, worktools, ConnectorError, SyncOutput};

/// Providers that sync over the network.
pub const ONLINE_PROVIDERS: &[&str] = &["google", "microsoft", "gdrive", "onedrive", "teams", "slack", "jira", "asana", "todoist", "linear"];
use crate::db::{self, audit, new_id, now, opt_ts, ts};
use crate::extract::{self, ExtractContext, Extracted};
use crate::ingest::{self, ApplyStats, Prepared};
use crate::models::{Account, Toast};
use crate::prefs::{self, Preferences};
use crate::secrets::{self, SecretStore};
use crate::{meetings, notify, planner, text};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub struct Agent {
    db: Mutex<Connection>,
    pub secrets: Arc<dyn SecretStore>,
    pub http: reqwest::Client,
    pub data_dir: PathBuf,
    rt: Mutex<Runtime>,
    /// Serialises sync/processing so overlapping ticks can't double-process.
    work: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Runtime {
    last_tick: Option<DateTime<Utc>>,
    ai_backoff_until: Option<DateTime<Utc>>,
    last_retention: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct TickReport {
    pub resumed_after_gap_minutes: Option<i64>,
    pub synced_accounts: usize,
    pub processed_items: usize,
    pub stats: ApplyStatsSer,
    pub notifications_created: usize,
    pub toasts: Vec<Toast>,
    pub errors: Vec<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct ApplyStatsSer {
    pub created: usize,
    pub updated: usize,
    pub for_review: usize,
    pub completed: usize,
    pub waiting_for: usize,
}

impl ApplyStatsSer {
    fn add(&mut self, s: &ApplyStats) {
        self.created += s.created;
        self.updated += s.updated;
        self.for_review += s.for_review;
        self.completed += s.completed;
        self.waiting_for += s.waiting_for;
    }
}

const ACCOUNT_COLS: &str = "id, provider, category, display_name, email, status, status_detail, scopes, enabled_streams, connected_at, last_sync_at, last_success_at, next_sync_at, backoff_until, error_count, items_synced";

fn account_row(r: &rusqlite::Row) -> rusqlite::Result<Account> {
    let streams: String = r.get(8)?;
    Ok(Account {
        id: r.get(0)?,
        provider: r.get(1)?,
        category: r.get(2)?,
        display_name: r.get(3)?,
        email: r.get(4)?,
        status: r.get(5)?,
        status_detail: r.get(6)?,
        scopes: r.get(7)?,
        enabled_streams: serde_json::from_str(&streams).unwrap_or_default(),
        connected_at: opt_ts(r.get(9)?),
        last_sync_at: opt_ts(r.get(10)?),
        last_success_at: opt_ts(r.get(11)?),
        next_sync_at: opt_ts(r.get(12)?),
        backoff_until: opt_ts(r.get(13)?),
        error_count: r.get(14)?,
        items_synced: r.get(15)?,
    })
}

impl Agent {
    pub fn open(data_dir: &Path, secrets: Arc<dyn SecretStore>) -> Result<Arc<Agent>> {
        let conn = db::open(&data_dir.join("kept.db"))?;
        Ok(Arc::new(Agent {
            db: Mutex::new(conn),
            secrets,
            http: connectors::http::client(),
            data_dir: data_dir.to_path_buf(),
            rt: Mutex::new(Runtime::default()),
            work: tokio::sync::Mutex::new(()),
        }))
    }

    /// After an update with better detection rules, re-check messages that produced nothing.
    pub fn reanalyse_after_upgrade(&self) -> Result<usize> {
        self.db(|c| {
            let cur = prefs::get_state(c, "rules_version")?;
            if cur.as_deref() == Some(ingest::RULES_VERSION) {
                return Ok(0);
            }
            let days = prefs::load(c)?.initial_sync_days as i64;
            let n = if cur.is_some() || c.query_row("SELECT COUNT(*) FROM source_items", [], |r| r.get::<_, i64>(0))? > 0 {
                ingest::reanalyse_unproductive(c, days)?
            } else {
                0
            };
            prefs::set_state(c, "rules_version", ingest::RULES_VERSION)?;
            if n > 0 {
                tracing::info!("detection rules updated: re-analysing {n} recent messages");
            }
            Ok(n)
        })
    }

    pub fn reanalyse(&self, days: i64) -> Result<usize> {
        self.db(|c| ingest::reanalyse_unproductive(c, days))
    }

    pub fn in_memory(secrets: Arc<dyn SecretStore>) -> Result<Arc<Agent>> {
        Ok(Arc::new(Agent {
            db: Mutex::new(db::open_in_memory()?),
            secrets,
            http: connectors::http::client(),
            data_dir: std::env::temp_dir(),
            rt: Mutex::new(Runtime::default()),
            work: tokio::sync::Mutex::new(()),
        }))
    }

    /// Run `f` with the database. Never hold this across an `.await`.
    pub fn db<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let c = self.db.lock().map_err(|_| anyhow!("database lock poisoned"))?;
        f(&c)
    }

    pub fn prefs(&self) -> Result<Preferences> {
        self.db(|c| prefs::load(c))
    }

    pub fn save_prefs(&self, p: &Preferences) -> Result<()> {
        self.db(|c| prefs::save(c, p))
    }

    // ------------------------------------------------------------------ accounts

    pub fn accounts(&self) -> Result<Vec<Account>> {
        self.db(|c| {
            let mut st = c.prepare(&format!("SELECT {ACCOUNT_COLS} FROM accounts ORDER BY connected_at"))?;
            let v = st.query_map([], account_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(v)
        })
    }

    fn account(&self, id: &str) -> Result<Account> {
        self.db(|c| Ok(c.query_row(&format!("SELECT {ACCOUNT_COLS} FROM accounts WHERE id=?1"), [id], account_row)?))
    }

    fn upsert_account(&self, provider: &str, category: &str, name: &str, email: Option<&str>, scopes: Option<&str>, streams: &[&str]) -> Result<String> {
        self.db(|c| {
            let existing: Option<String> = c
                .query_row("SELECT id FROM accounts WHERE provider=?1 AND COALESCE(email,'')=COALESCE(?2,'')", params![provider, email], |r| r.get(0))
                .optional()?;
            let id = existing.unwrap_or_else(new_id);
            c.execute(
                "INSERT INTO accounts(id, provider, category, display_name, email, status, scopes, enabled_streams, connected_at, next_sync_at)
                 VALUES (?1,?2,?3,?4,?5,'connected',?6,?7,?8,?8)
                 ON CONFLICT(id) DO UPDATE SET status='connected', status_detail=NULL, scopes=excluded.scopes, error_count=0, backoff_until=NULL, next_sync_at=excluded.next_sync_at",
                params![id, provider, category, name, email, scopes, serde_json::to_string(streams)?, ts(&now())],
            )?;
            audit(c, "account", &id, "connected", None, Some(provider), "user", "")?;
            Ok(id)
        })
    }

    fn set_account_status(&self, id: &str, status: &str, detail: Option<&str>) -> Result<()> {
        self.db(|c| {
            c.execute("UPDATE accounts SET status=?1, status_detail=?2 WHERE id=?3", params![status, detail, id])?;
            Ok(())
        })
    }

    fn google_client(&self, p: &Preferences) -> Result<(String, Option<String>)> {
        let id = if !p.google_client_id.trim().is_empty() { p.google_client_id.trim().to_string() } else { option_env!("KEPT_GOOGLE_CLIENT_ID").unwrap_or("").to_string() };
        if id.is_empty() {
            bail!("No Google OAuth client is configured. Add your Google Cloud \"Desktop app\" client ID in Integrations → Google → Setup (see docs/SETUP.md).");
        }
        let secret = self.secrets.get(&secrets::client_secret_key_name("google"))?.or_else(|| option_env!("KEPT_GOOGLE_CLIENT_SECRET").map(|s| s.to_string()));
        Ok((id, secret))
    }

    fn microsoft_client(&self, p: &Preferences) -> Result<(String, String)> {
        let id = if !p.microsoft_client_id.trim().is_empty() { p.microsoft_client_id.trim().to_string() } else { option_env!("KEPT_MS_CLIENT_ID").unwrap_or("").to_string() };
        if id.is_empty() {
            bail!("No Microsoft app registration is configured. Add your Azure application (client) ID in Integrations → Microsoft → Setup (see docs/SETUP.md).");
        }
        Ok((id, p.microsoft_tenant.clone()))
    }

    fn slack_client(&self, p: &Preferences) -> Result<String> {
        let id = if !p.slack_client_id.trim().is_empty() { p.slack_client_id.trim().to_string() } else { option_env!("KEPT_SLACK_CLIENT_ID").unwrap_or("").to_string() };
        if id.is_empty() {
            bail!("No Slack app is configured. Create a Slack app and add its Client ID in Integrations → Slack → App setup (see docs/SETUP.md).");
        }
        Ok(id)
    }

    /// OAuth endpoints for a provider (used for sign-in and for refreshing tokens).
    fn endpoints_for(&self, provider: &str, p: &Preferences) -> Result<oauth::ProviderEndpoints> {
        Ok(match provider {
            "google" => {
                let (id, secret) = self.google_client(p)?;
                google::endpoints(&id, secret)
            }
            "gdrive" => {
                let (id, secret) = self.google_client(p)?;
                google::endpoints_with(&id, secret, docs::GDRIVE_SCOPES)
            }
            "microsoft" => {
                let (id, tenant) = self.microsoft_client(p)?;
                microsoft::endpoints(&id, &tenant)
            }
            "onedrive" => {
                let (id, tenant) = self.microsoft_client(p)?;
                microsoft::endpoints_with(&id, &tenant, docs::ONEDRIVE_SCOPES)
            }
            "teams" => {
                let (id, tenant) = self.microsoft_client(p)?;
                teams::endpoints(&id, &tenant)
            }
            "slack" => slack::endpoints(&self.slack_client(p)?),
            other => bail!("{other} does not use browser sign-in"),
        })
    }

    /// Interactive sign-in. `open_browser` must open the URL in the system browser.
    pub async fn connect(&self, provider: &str, open_browser: impl Fn(&str) -> Result<()>) -> Result<Account> {
        let p = self.prefs()?;
        let ep = self.endpoints_for(provider, &p)?;
        let pending = oauth::begin(&ep).await.map_err(|e| anyhow!(e.to_string()))?;
        open_browser(&pending.auth_url)?;
        let code = oauth::wait_for_code(&pending, std::time::Duration::from_secs(300)).await.map_err(|e| anyhow!(e.to_string()))?;
        let tokens = oauth::exchange(&self.http, &ep, &pending, &code).await.map_err(|e| anyhow!("token exchange failed: {e}"))?;
        let mut aliases: Vec<String> = vec![];
        let mut config = serde_json::json!({});
        let (email, name) = match provider {
            "google" | "gdrive" => google::whoami(&self.http, &tokens.access_token).await,
            "slack" => slack::whoami(&self.http, &tokens.access_token).await.map(|me| {
                aliases = me.handles.clone();
                config = serde_json::json!({"user_id": me.user_id, "team": me.team, "team_url": me.team_url, "name": me.name});
                (me.email.clone().unwrap_or_else(|| format!("{}@slack", me.user_id)), me.name.clone())
            }),
            _ => microsoft::whoami(&self.http, &tokens.access_token).await,
        }
        .map_err(|e| anyhow!("could not read account profile: {e}"))?;
        let (display, category, streams): (String, &str, &[&str]) = match provider {
            "google" => (format!("Google · {email}"), "multi", &["mail", "calendar"]),
            "gdrive" => (format!("Google Drive · {email}"), "documents", &["documents"]),
            "onedrive" => (format!("OneDrive & SharePoint · {email}"), "documents", &["documents"]),
            "teams" => (format!("Microsoft Teams · {email}"), "chat", &["chat"]),
            "slack" => (format!("Slack · {} · {email}", config["team"].as_str().unwrap_or("workspace")), "chat", &["chat"]),
            _ => (format!("Microsoft 365 · {email}"), "multi", &["mail", "calendar"]),
        };
        let id = self.upsert_account(provider, category, &display, Some(&email), tokens.scope.as_deref().or(Some(&ep.scopes.join(" "))), streams)?;
        self.db(|c| {
            c.execute("UPDATE accounts SET config_json=?1 WHERE id=?2", params![config.to_string(), id])?;
            Ok(())
        })?;
        self.secrets.set(&secrets::token_key_name(&id), &serde_json::to_string(&tokens)?).context("saving token to the OS credential store")?;
        self.remember_identity(&email, name, &aliases)?;
        tracing::info!("connected {provider} account");
        self.account(&id)
    }

    /// Ownership detection depends on knowing every address and handle that is "you".
    fn remember_identity(&self, email: &str, name: Option<String>, aliases: &[String]) -> Result<()> {
        let mut p = self.prefs()?;
        if !email.is_empty() && !p.is_me(email) {
            p.user_emails.push(email.to_lowercase());
        }
        for a in aliases {
            let a = a.trim().trim_start_matches('@').to_string();
            if a.len() >= 2 && !p.user_aliases.iter().any(|x| x.eq_ignore_ascii_case(&a)) && !p.user_name.eq_ignore_ascii_case(&a) {
                p.user_aliases.push(a);
            }
        }
        if p.user_name.is_empty() {
            if let Some(n) = name {
                p.user_name = n;
            }
        }
        self.save_prefs(&p)
    }

    /// Connect a task tool with its personal API token. The token is verified, then stored
    /// only in the OS credential store. fields: token (+ email and site for Jira).
    pub async fn connect_with_token(&self, provider: &str, token: &str, email: Option<&str>, site: Option<&str>) -> Result<Account> {
        let sec = worktools::TokenSecret { token: token.trim().to_string(), email: email.map(|e| e.trim().to_string()).filter(|e| !e.is_empty()), site: site.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) };
        if sec.token.len() < 8 || sec.token.chars().any(|c| c.is_control() || c.is_whitespace()) {
            bail!("That doesn't look like a valid API token.");
        }
        let who = match provider {
            "jira" => {
                if sec.email.is_none() || sec.site.is_none() {
                    bail!("Jira needs your site address, account email and API token.");
                }
                worktools::jira_whoami(&self.http, &sec).await
            }
            "asana" => worktools::asana_whoami(&self.http, &sec).await,
            "todoist" => worktools::todoist_whoami(&self.http, &sec).await,
            "linear" => worktools::linear_whoami(&self.http, &sec).await,
            other => bail!("{other} does not use API tokens"),
        }
        .map_err(|e| match e {
            ConnectorError::Auth => anyhow!("The token was rejected. Check it and try again."),
            other => anyhow!("Could not verify the token: {other}"),
        })?;
        let id = self.upsert_account(provider, "tasks", &who.label, who.email.as_deref(), Some("personal API token (read; Todoist write only on confirmation)"), &["tasks"])?;
        self.secrets.set(&secrets::token_key_name(&id), &serde_json::to_string(&sec)?).context("saving token to the OS credential store")?;
        if let Some(e) = &who.email {
            self.remember_identity(e, who.name.clone(), &[])?;
        }
        self.account(&id)
    }

    fn token_secret(&self, acct: &Account) -> Result<worktools::TokenSecret, ConnectorError> {
        let raw = self.secrets.get(&secrets::token_key_name(&acct.id)).map_err(|e| ConnectorError::Permanent(e.to_string()))?.ok_or(ConnectorError::Auth)?;
        serde_json::from_str(&raw).map_err(|_| ConnectorError::Auth)
    }

    async fn access_token(&self, acct: &Account) -> Result<String, ConnectorError> {
        let raw = self.secrets.get(&secrets::token_key_name(&acct.id)).map_err(|e| ConnectorError::Permanent(e.to_string()))?.ok_or(ConnectorError::Auth)?;
        let t: oauth::TokenSet = serde_json::from_str(&raw).map_err(|_| ConnectorError::Auth)?;
        if t.fresh() || (t.refresh_token.is_none() && acct.provider == "slack") {
            return Ok(t.access_token);
        }
        let p = self.prefs().map_err(|e| ConnectorError::Permanent(e.to_string()))?;
        let ep = self.endpoints_for(&acct.provider, &p).map_err(|e| ConnectorError::Permanent(e.to_string()))?;
        let nt = oauth::refresh(&self.http, &ep, &t).await?;
        self.secrets
            .set(&secrets::token_key_name(&acct.id), &serde_json::to_string(&nt).unwrap_or_default())
            .map_err(|e| ConnectorError::Permanent(e.to_string()))?;
        Ok(nt.access_token)
    }

    fn cursor(&self, account: &str, stream: &str) -> Result<Option<String>> {
        self.db(|c| Ok(c.query_row("SELECT cursor FROM sync_cursors WHERE account_id=?1 AND stream=?2", params![account, stream], |r| r.get(0)).optional()?))
    }

    fn account_config(&self, id: &str) -> serde_json::Value {
        self.db(|c| Ok(c.query_row("SELECT config_json FROM accounts WHERE id=?1", [id], |r| r.get::<_, String>(0))?))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(serde_json::Value::Null)
    }

    async fn fetch(&self, acct: &Account, initial_days: u32) -> Result<SyncOutput, ConnectorError> {
        let mut out = SyncOutput::default();
        let streams = &acct.enabled_streams;
        let on = |s: &str| streams.iter().any(|x| x == s);
        let perm = |e: anyhow::Error| ConnectorError::Permanent(e.to_string());
        match acct.provider.as_str() {
            "jira" | "asana" | "todoist" | "linear" => {
                if on("tasks") {
                    let sec = self.token_secret(acct)?;
                    match acct.provider.as_str() {
                        "jira" => worktools::jira_sync(&self.http, &sec, &mut out).await?,
                        "asana" => worktools::asana_sync(&self.http, &sec, &mut out).await?,
                        "todoist" => worktools::todoist_sync(&self.http, &sec, &mut out).await?,
                        _ => worktools::linear_sync(&self.http, &sec, &mut out).await?,
                    }
                }
                return Ok(out);
            }
            _ => {}
        }
        let token = self.access_token(acct).await?;
        match acct.provider.as_str() {
            "google" => {
                if on("mail") {
                    let cur = self.cursor(&acct.id, "mail").map_err(perm)?;
                    google::sync_mail(&self.http, &token, cur.as_deref(), initial_days, &mut out).await?;
                }
                if on("calendar") {
                    google::sync_calendar(&self.http, &token, &mut out).await?;
                }
            }
            "microsoft" => {
                if on("mail") {
                    for (folder, stream) in [("inbox", "mail"), ("sentitems", "sent")] {
                        let cur = self.cursor(&acct.id, stream).map_err(perm)?;
                        microsoft::sync_folder(&self.http, &token, folder, stream, cur.as_deref(), initial_days, &mut out).await?;
                    }
                }
                if on("calendar") {
                    microsoft::sync_calendar(&self.http, &token, &mut out).await?;
                }
            }
            "gdrive" if on("documents") => docs::sync_gdrive(&self.http, &token, &mut out).await?,
            "onedrive" if on("documents") => docs::sync_onedrive(&self.http, &token, &mut out).await?,
            "teams" if on("chat") => {
                let cur = self.cursor(&acct.id, "chat").map_err(perm)?;
                let email = acct.email.clone().unwrap_or_default();
                teams::sync(&self.http, &token, &email, cur.as_deref(), initial_days, &mut out).await?;
            }
            "slack" if on("chat") => {
                let cfg = self.account_config(&acct.id);
                let me = slack::Me {
                    user_id: cfg["user_id"].as_str().unwrap_or_default().to_string(),
                    email: acct.email.clone(),
                    name: cfg["name"].as_str().map(|s| s.to_string()),
                    handles: vec![],
                    team: cfg["team"].as_str().unwrap_or_default().to_string(),
                    team_url: cfg["team_url"].as_str().unwrap_or("https://slack.com/").to_string(),
                };
                if me.user_id.is_empty() {
                    return Err(ConnectorError::Auth);
                }
                let cur = self.cursor(&acct.id, "chat").map_err(perm)?;
                let email = acct.email.clone().unwrap_or_default();
                slack::sync(&self.http, &token, &me, &email, cur.as_deref(), initial_days, &mut out).await?;
            }
            _ => {}
        }
        Ok(out)
    }

    /// Sync one account now. Errors are recorded on the account, not thrown away.
    pub async fn sync_account(&self, id: &str) -> Result<String> {
        let acct = self.account(id)?;
        if !ONLINE_PROVIDERS.contains(&acct.provider.as_str()) {
            return Ok("Local data — nothing to sync".into());
        }
        let p = self.prefs()?;
        self.set_account_status(id, "syncing", None)?;
        let result = self.fetch(&acct, p.initial_sync_days).await;
        let n = now();
        match result {
            Ok(out) => {
                let summary = self.db(|c| {
                    let m = ingest::store_messages(c, id, &acct.provider, &p, &out.messages)?;
                    let e = ingest::store_events(c, id, &out.events)?;
                    let d = ingest::store_documents(c, id, &acct.provider, &out.documents)?;
                    let x = match &out.external_tasks {
                        Some(items) => Some(ingest::store_external_tasks(c, &p, id, &acct.provider, items, !out.snapshot_incomplete)?),
                        None => None,
                    };
                    let tx = c.unchecked_transaction()?;
                    for s in &out.reset {
                        tx.execute("DELETE FROM sync_cursors WHERE account_id=?1 AND stream=?2", params![id, s])?;
                    }
                    for (s, cur) in &out.cursors {
                        tx.execute(
                            "INSERT INTO sync_cursors(account_id, stream, cursor, updated_at) VALUES (?1,?2,?3,?4)
                             ON CONFLICT(account_id, stream) DO UPDATE SET cursor=excluded.cursor, updated_at=excluded.updated_at",
                            params![id, s, cur, ts(&n)],
                        )?;
                    }
                    let detail = if out.notes.is_empty() { None } else { Some(out.notes.join("; ")) };
                    tx.execute(
                        "UPDATE accounts SET status='connected', status_detail=?1, last_sync_at=?2, last_success_at=?2, error_count=0, backoff_until=NULL, next_sync_at=?3 WHERE id=?4",
                        params![detail, ts(&n), ts(&(n + Duration::minutes(p.sync_interval_minutes as i64))), id],
                    )?;
                    if let Some(items) = &out.external_tasks {
                        tx.execute("UPDATE accounts SET items_synced=?1 WHERE id=?2", params![items.len() as i64, id])?;
                    }
                    if !out.documents.is_empty() {
                        tx.execute("UPDATE accounts SET items_synced=(SELECT COUNT(*) FROM documents WHERE account_id=?1) WHERE id=?1", params![id])?;
                    }
                    tx.commit()?;
                    Ok(match (acct.provider.as_str(), x) {
                        (_, Some(x)) => format!("{} new tasks, {} linked to detected tasks, {} closed, {} updated", x.created, x.linked, x.closed, x.updated),
                        ("gdrive" | "onedrive", _) => format!("{} documents indexed", d.new_items),
                        ("slack" | "teams", _) => format!("{} new messages", m.new_items),
                        _ => format!("{} new messages, {} events updated", m.new_items, e.new_items + e.updated_items),
                    })
                })?;
                Ok(summary)
            }
            Err(err) => {
                let (status, detail, backoff) = match &err {
                    ConnectorError::Auth => ("attention", "Sign-in expired or access was revoked. Reconnect this account.".to_string(), Duration::days(365)),
                    ConnectorError::RateLimited(s) => ("connected", format!("Rate limited by provider; retrying in {} min", (s / 60).max(1)), Duration::seconds(*s as i64)),
                    ConnectorError::Transient(m) => {
                        let mins = 2i64.pow(acct.error_count.clamp(0, 5) as u32 + 1).min(60);
                        ("connected", format!("Offline or provider unreachable ({}); retrying in {mins} min", text::truncate(m, 80)), Duration::minutes(mins))
                    }
                    ConnectorError::CursorExpired => ("connected", "Sync position expired; doing a fresh sync".to_string(), Duration::minutes(1)),
                    ConnectorError::Permanent(m) => ("error", text::truncate(m, 200), Duration::minutes(60)),
                };
                self.db(|c| {
                    if matches!(err, ConnectorError::CursorExpired) {
                        c.execute("DELETE FROM sync_cursors WHERE account_id=?1", [id])?;
                    }
                    c.execute(
                        "UPDATE accounts SET status=?1, status_detail=?2, last_sync_at=?3, error_count=error_count+1, backoff_until=?4, next_sync_at=?4 WHERE id=?5",
                        params![status, detail, ts(&n), ts(&(n + backoff)), id],
                    )?;
                    Ok(())
                })?;
                tracing::warn!("sync {} failed: {}", acct.provider, err);
                Err(anyhow!(detail))
            }
        }
    }

    /// Disconnect: revoke where supported, delete tokens, and remove synced content.
    /// `delete_tasks`: also delete unconfirmed suggestions that only came from this account,
    /// and tasks mirrored from a task tool.
    pub async fn disconnect(&self, id: &str, delete_tasks: bool) -> Result<()> {
        let acct = self.account(id)?;
        if let Ok(Some(raw)) = self.secrets.get(&secrets::token_key_name(id)) {
            if let Ok(t) = serde_json::from_str::<oauth::TokenSet>(&raw) {
                match acct.provider.as_str() {
                    "google" | "gdrive" => google::revoke(&self.http, t.refresh_token.as_deref().unwrap_or(&t.access_token)).await,
                    "slack" => {
                        let _ = self.http.get("https://slack.com/api/auth.revoke").bearer_auth(&t.access_token).send().await;
                    }
                    // Microsoft has no per-token revocation for public clients; the user can remove
                    // consent at https://myapps.microsoft.com (documented in the UI).
                    _ => {}
                }
            }
            // Personal API tokens (Jira, Asana, Todoist, Linear) are deleted here; the UI tells
            // the user where to revoke them in the tool itself.
        }
        let _ = self.secrets.delete(&secrets::token_key_name(id));
        self.db(|c| {
            let tx = c.unchecked_transaction()?;
            if delete_tasks {
                tx.execute(
                    "DELETE FROM tasks WHERE id IN (
                        SELECT t.id FROM tasks t WHERE (t.needs_confirmation=1 OR t.origin <> 'manual' AND ?2 = 'sample')
                          AND NOT EXISTS (SELECT 1 FROM task_sources ts LEFT JOIN source_items si ON si.id=ts.source_item_id
                                          WHERE ts.task_id=t.id AND (si.account_id IS NULL OR si.account_id <> ?1))
                          AND EXISTS (SELECT 1 FROM task_sources ts JOIN source_items si ON si.id=ts.source_item_id WHERE ts.task_id=t.id AND si.account_id=?1))",
                    params![id, acct.provider],
                )?;
                tx.execute(
                    "DELETE FROM tasks WHERE origin='external' AND id IN (SELECT task_id FROM external_tasks WHERE account_id=?1 AND task_id IS NOT NULL)",
                    [id],
                )?;
            }
            tx.execute("DELETE FROM source_items WHERE account_id=?1", [id])?;
            tx.execute("DELETE FROM meetings WHERE account_id=?1", [id])?;
            tx.execute("DELETE FROM documents WHERE account_id=?1", [id])?;
            tx.execute("DELETE FROM external_tasks WHERE account_id=?1", [id])?;
            tx.execute("DELETE FROM sync_cursors WHERE account_id=?1", [id])?;
            tx.execute("DELETE FROM accounts WHERE id=?1", [id])?;
            audit(&tx, "account", id, "disconnected", Some(&acct.provider), None, "user", if delete_tasks { "with task cleanup" } else { "" })?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Create a task in Todoist from a Kept task. Only call after the user confirmed the
    /// destination, content and effect shown in the app. Recorded in `external_actions`.
    pub async fn push_to_todoist(&self, task_id: &str, account_id: Option<&str>) -> Result<String> {
        let acct = match account_id {
            Some(a) => self.account(a)?,
            None => self.accounts()?.into_iter().find(|a| a.provider == "todoist").ok_or_else(|| anyhow!("Connect Todoist in Integrations first."))?,
        };
        anyhow::ensure!(acct.provider == "todoist", "That account is not Todoist");
        let t = self.db(|c| crate::tasks::get(c, task_id))?;
        let already: Option<String> = self.db(|c| Ok(c.query_row("SELECT url FROM external_tasks WHERE task_id=?1 AND provider='todoist'", [task_id], |r| r.get(0)).optional()?))?;
        if let Some(u) = already {
            bail!("This task is already in Todoist ({}).", u);
        }
        let sec = self.token_secret(&acct).map_err(|e| anyhow!(e.to_string()))?;
        let desc = format!("{}Added from Kept.", t.description.clone().map(|d| format!("{d}\n\n")).unwrap_or_default());
        let action_id = new_id();
        let payload = serde_json::json!({"content": t.title, "description": desc, "due": t.due_at});
        self.db(|c| {
            c.execute(
                "INSERT INTO external_actions(id, account_id, action, destination, payload, effect, state, created_at, decided_at) VALUES (?1,?2,'create_external_task',?3,?4,?5,'confirmed',?6,?6)",
                params![action_id, acct.id, acct.display_name, payload.to_string(), "Creates one new task in Todoist", ts(&now())],
            )?;
            Ok(())
        })?;
        match worktools::todoist_create(&self.http, &sec, &t.title, Some(&desc), t.due_at).await {
            Ok((xid, url)) => {
                self.db(|c| {
                    c.execute("UPDATE external_actions SET state='executed' WHERE id=?1", [&action_id])?;
                    c.execute(
                        "INSERT OR IGNORE INTO external_tasks(id, account_id, provider, external_id, title, status, is_done, due_at, url, task_id, updated_at) VALUES (?1,?2,'todoist',?3,?4,'Open',0,?5,?6,?7,?8)",
                        params![new_id(), acct.id, xid, t.title, t.due_at.as_ref().map(ts), url, task_id, ts(&now())],
                    )?;
                    crate::tasks::add_source(c, task_id, &crate::tasks::SourceLink { source_item_id: None, meeting_id: None, relation: "external", quote: Some(&t.title), provider: Some("todoist"), url: url.as_deref(), label: Some("Todoist · added from Kept".into()) })?;
                    audit(c, "task", task_id, "pushed", None, Some("todoist"), "user", "created in Todoist after confirmation")?;
                    Ok(())
                })?;
                Ok("Added to Todoist.".into())
            }
            Err(e) => {
                let _ = self.db(|c| {
                    c.execute("UPDATE external_actions SET state='failed' WHERE id=?1", [&action_id])?;
                    Ok(())
                });
                Err(anyhow!("Todoist didn't accept the task: {e}"))
            }
        }
    }

    pub fn set_streams(&self, id: &str, streams: &[String]) -> Result<()> {
        self.db(|c| {
            c.execute("UPDATE accounts SET enabled_streams=?1 WHERE id=?2", params![serde_json::to_string(streams)?, id])?;
            Ok(())
        })
    }

    // ------------------------------------------------------------------ imports

    fn local_account(&self, provider: &str, name: &str) -> Result<String> {
        self.db(|c| {
            if let Some(id) = c.query_row("SELECT id FROM accounts WHERE provider=?1", [provider], |r| r.get::<_, String>(0)).optional()? {
                return Ok(id);
            }
            let id = new_id();
            c.execute(
                "INSERT INTO accounts(id, provider, category, display_name, status, enabled_streams, connected_at, last_success_at) VALUES (?1,?2,'multi',?3,'connected','[]',?4,?4)",
                params![id, provider, name, ts(&now())],
            )?;
            Ok(id)
        })
    }

    pub fn import_files(&self, paths: &[PathBuf]) -> Result<String> {
        let p = self.prefs()?;
        let acct = self.local_account("import", "Imported files")?;
        let (mut msgs, mut evs) = (vec![], vec![]);
        let mut errors = vec![];
        for path in paths {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let meta = std::fs::metadata(path)?;
            if meta.len() > 25 * 1024 * 1024 {
                errors.push(format!("{name}: larger than 25 MB, skipped"));
                continue;
            }
            match ext.as_str() {
                "ics" => match files::parse_ics(&std::fs::read_to_string(path)?, p.tz()) {
                    Ok(v) => evs.extend(v),
                    Err(e) => errors.push(format!("{name}: {e}")),
                },
                "eml" => match files::parse_eml(&std::fs::read(path)?, &name) {
                    Ok(m) => msgs.push(m),
                    Err(e) => errors.push(e.to_string()),
                },
                "json" => match files::parse_bundle(&std::fs::read_to_string(path)?) {
                    Ok(b) => {
                        msgs.extend(b.messages);
                        evs.extend(b.events);
                    }
                    Err(e) => errors.push(format!("{name}: {e}")),
                },
                _ => errors.push(format!("{name}: unsupported file type (use .eml, .ics or .json)")),
            }
        }
        let (m, e) = self.db(|c| Ok((ingest::store_messages(c, &acct, "import", &p, &msgs)?, ingest::store_events(c, &acct, &evs)?)))?;
        let _ = self.db(meetings::refresh_prep_states);
        let mut s = format!("Imported {} messages and {} events.", m.new_items, e.new_items + e.updated_items);
        if !errors.is_empty() {
            s.push_str(&format!(" Skipped: {}", errors.join("; ")));
        }
        Ok(s)
    }

    pub fn load_sample(&self) -> Result<String> {
        let mut p = self.prefs()?;
        if p.user_emails.is_empty() {
            p.user_emails.push("alex@example.com".into());
            if p.user_name.is_empty() {
                p.user_name = "Alex Morgan".into();
            }
            self.save_prefs(&p)?;
        }
        let acct = self.local_account("sample", "Sample workspace")?;
        let (msgs, evs) = sample::build(&p, now());
        self.db(|c| {
            ingest::store_messages(c, &acct, "sample", &p, &msgs)?;
            ingest::store_events(c, &acct, &evs)?;
            meetings::refresh_prep_states(c)
        })?;
        Ok(format!("Loaded {} sample messages and {} meetings.", msgs.len(), evs.len()))
    }

    // ------------------------------------------------------------------ AI

    pub fn ai_config(&self, p: &Preferences) -> Option<AiConfig> {
        if p.extraction_mode == "rules_only" || p.ai.provider == "none" || p.ai.provider.is_empty() {
            return None;
        }
        let key = self.secrets.get(&secrets::ai_key_name(&p.ai.provider)).ok().flatten();
        if key.is_none() && p.ai.provider != "openai_compatible" {
            return None;
        }
        Some(AiConfig {
            provider: p.ai.provider.clone(),
            model: p.ai.model.clone(),
            base_url: p.ai.base_url.clone(),
            api_key: key,
            timeout: std::time::Duration::from_secs(p.ai.timeout_seconds.clamp(5, 120)),
        })
    }

    pub async fn ai_test(&self) -> Result<String> {
        let p = self.prefs()?;
        let cfg = self.ai_config(&p).ok_or_else(|| anyhow!("Choose a provider and save an API key first."))?;
        ai::test(&self.http, &cfg).await.map_err(|e| anyhow!(e.to_string()))
    }

    fn ai_allowed_today(&self, p: &Preferences) -> Result<bool> {
        let since = ts(&(now() - Duration::hours(24)));
        let n: i64 = self.db(|c| Ok(c.query_row("SELECT COUNT(*) FROM ai_runs WHERE created_at > ?1", [since], |r| r.get(0))?))?;
        Ok(n < p.ai.daily_request_limit as i64)
    }

    fn record_ai(&self, cfg: &AiConfig, purpose: &str, source_ids: &[&str], req: usize, resp: usize, items: usize, rejected: usize, err: Option<&str>, ms: u128) -> Result<()> {
        self.db(|c| {
            c.execute(
                "INSERT INTO ai_runs(id, provider, model, purpose, source_ids, request_chars, response_chars, items_returned, items_rejected, ok, error, duration_ms, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![new_id(), cfg.provider, cfg.model(), purpose, serde_json::to_string(source_ids)?, req as i64, resp as i64, items as i64, rejected as i64, err.is_none(), err, ms as i64, ts(&now())],
            )?;
            Ok(())
        })
    }

    async fn extract_with_ai(&self, cfg: &AiConfig, p: &Preferences, prep: &Prepared) -> Result<Vec<Extracted>, AiError> {
        let ctx = ExtractContext { prefs: p, item: &prep.item, thread: &prep.thread, open_tasks: &prep.open_tasks, now: now() };
        let (user, n) = extract::ai::build_user_prompt(&ctx);
        let started = std::time::Instant::now();
        let mut res = ai::complete(&self.http, cfg, extract::ai::SYSTEM_PROMPT, &user, 1500).await;
        // One repair attempt on malformed output.
        let parsed = match &res {
            Ok(t) => ai::extract_json(t).ok(),
            Err(_) => None,
        };
        let parsed = match (parsed, &res) {
            (Some(v), _) => Some(v),
            (None, Ok(_)) => {
                res = ai::complete(&self.http, cfg, extract::ai::SYSTEM_PROMPT, &format!("{user}\n\nYour previous reply was not valid JSON. Reply with ONLY the JSON object."), 1500).await;
                res.as_ref().ok().and_then(|t| ai::extract_json(t).ok())
            }
            _ => None,
        };
        let ms = started.elapsed().as_millis();
        match (parsed, res) {
            (Some(v), Ok(t)) => {
                let val = extract::ai::validate(&v, &ctx);
                if val.injection_suspected {
                    tracing::warn!("possible prompt injection in source item {}", prep.item.id);
                }
                let _ = self.record_ai(cfg, "extract", &[&prep.item.id], n, t.len(), val.items.len(), val.rejected, None, ms);
                Ok(val.items)
            }
            (_, Err(e)) => {
                let _ = self.record_ai(cfg, "extract", &[&prep.item.id], n, 0, 0, 0, Some(&e.to_string()), ms);
                Err(e)
            }
            (None, Ok(t)) => {
                let _ = self.record_ai(cfg, "extract", &[&prep.item.id], n, t.len(), 0, 0, Some("malformed JSON"), ms);
                Err(AiError::Malformed("not JSON after retry".into()))
            }
        }
    }

    fn ai_permitted_for(p: &Preferences, kind: &str) -> bool {
        match kind {
            "chat" => p.ai.process_chat,
            _ => p.ai.process_email,
        }
    }

    /// Extract from pending items: AI when configured & permitted, rules otherwise / on failure.
    pub async fn process_pending(&self, max: i64) -> Result<(usize, ApplyStatsSer)> {
        let p = self.prefs()?;
        let batch = self.db(|c| ingest::prepare_batch(c, max))?;
        let mut stats = ApplyStatsSer::default();
        let cfg = self.ai_config(&p);
        let mut done = 0;
        for prep in batch {
            let backoff = self.rt.lock().unwrap().ai_backoff_until.map(|u| u > now()).unwrap_or(false);
            let use_ai = cfg.is_some() && !backoff && Self::ai_permitted_for(&p, &prep.item.kind) && self.ai_allowed_today(&p)?;
            let (items, note) = if use_ai {
                match self.extract_with_ai(cfg.as_ref().unwrap(), &p, &prep).await {
                    Ok(v) => (v, "ai".to_string()),
                    Err(e) => {
                        if matches!(e, AiError::RateLimited(_) | AiError::Auth | AiError::Timeout | AiError::Network(_)) {
                            let secs = if let AiError::RateLimited(Some(s)) = e { s as i64 } else { 300 };
                            self.rt.lock().unwrap().ai_backoff_until = Some(now() + Duration::seconds(secs));
                        }
                        (ingest::extract_rules(&p, &prep), format!("rules (AI failed: {e})"))
                    }
                }
            } else {
                (ingest::extract_rules(&p, &prep), "rules".into())
            };
            let s = self.db(|c| {
                let s = ingest::apply(c, &p, &prep.item, &items)?;
                ingest::mark_processed(c, &prep.item.id, "done", &format!("{note}: {} item(s)", items.len()))?;
                Ok(s)
            });
            match s {
                Ok(s) => stats.add(&s),
                Err(e) => {
                    tracing::error!("apply failed for {}: {e:#}", prep.item.id);
                    let _ = self.db(|c| ingest::mark_processed(c, &prep.item.id, "error", &text::truncate(&format!("{e:#}"), 200)));
                }
            }
            done += 1;
        }
        if done > 0 {
            let _ = self.db(|c| {
                planner::reprioritize_all(c, &p)?;
                meetings::refresh_prep_states(c)
            });
        }
        Ok((done, stats))
    }

    pub async fn ai_meeting_summary(&self, meeting_id: &str) -> Result<String> {
        let p = self.prefs()?;
        let cfg = self.ai_config(&p).ok_or_else(|| anyhow!("No AI provider configured. The briefing above is built without AI."))?;
        if !p.ai.process_calendar {
            bail!("AI processing of calendar data is turned off in Settings → Privacy.");
        }
        let b = self.db(|c| meetings::briefing(c, &p, meeting_id))?;
        let prompt = meetings::briefing_prompt(&b, p.ai.redact_pii);
        let started = std::time::Instant::now();
        let out = ai::complete(&self.http, &cfg, meetings::BRIEFING_SYSTEM, &prompt, 400).await;
        let ms = started.elapsed().as_millis();
        match out {
            Ok(t) => {
                let summary = ai::extract_json(&t).ok().and_then(|v| v["summary"].as_str().map(|s| s.to_string())).unwrap_or_else(|| text::truncate(&t, 800));
                let summary = text::truncate(&summary, 1200);
                self.record_ai(&cfg, "briefing", &[meeting_id], prompt.len(), t.len(), 1, 0, None, ms)?;
                self.db(|c| prefs::set_state(c, &format!("briefing_ai:{meeting_id}"), &summary))?;
                Ok(summary)
            }
            Err(e) => {
                self.record_ai(&cfg, "briefing", &[meeting_id], prompt.len(), 0, 0, 0, Some(&e.to_string()), ms)?;
                Err(anyhow!(e.to_string()))
            }
        }
    }

    // ------------------------------------------------------------------ the loop

    /// One iteration of the background loop. Call every ~60 s.
    pub async fn tick(&self) -> TickReport {
        let _guard = self.work.lock().await;
        let mut rep = TickReport::default();
        let at = now();
        {
            let mut rt = self.rt.lock().unwrap();
            if let Some(last) = rt.last_tick {
                let gap = (at - last).num_minutes();
                if gap >= 4 {
                    rep.resumed_after_gap_minutes = Some(gap);
                }
            }
            rt.last_tick = Some(at);
        }
        let p = match self.prefs() {
            Ok(p) => p,
            Err(e) => {
                rep.errors.push(e.to_string());
                return rep;
            }
        };
        if rep.resumed_after_gap_minutes.is_some() {
            // Woke from sleep / came back online: sync everything soon, reminders recalculate below.
            let _ = self.db(|c| {
                c.execute("UPDATE accounts SET next_sync_at=?1 WHERE status IN ('connected') AND (backoff_until IS NULL OR backoff_until < ?1)", [ts(&at)])?;
                Ok(())
            });
            tracing::info!("resumed after {:?} min gap", rep.resumed_after_gap_minutes);
        }
        if !p.monitoring_paused {
            let due: Vec<Account> = self
                .accounts()
                .unwrap_or_default()
                .into_iter()
                .filter(|a| ONLINE_PROVIDERS.contains(&a.provider.as_str()))
                .filter(|a| a.status != "attention")
                .filter(|a| a.next_sync_at.map(|n| n <= at).unwrap_or(true))
                .collect();
            for a in due {
                match self.sync_account(&a.id).await {
                    Ok(_) => rep.synced_accounts += 1,
                    Err(e) => rep.errors.push(format!("{}: {e}", a.display_name)),
                }
            }
            match self.process_pending(40).await {
                Ok((n, s)) => {
                    rep.processed_items = n;
                    rep.stats = s;
                }
                Err(e) => rep.errors.push(format!("processing: {e:#}")),
            }
        }
        let r = self.db(|c| {
            planner::reprioritize_all(c, &p)?;
            meetings::refresh_prep_states(c)?;
            let created = notify::generate(c, &p, at)?;
            let toasts = notify::due_toasts(c, &p, at)?;
            Ok((created, toasts))
        });
        match r {
            Ok((c, t)) => {
                rep.notifications_created = c;
                rep.toasts = t;
            }
            Err(e) => rep.errors.push(format!("notify: {e:#}")),
        }
        // Retention + housekeeping once every 12 h.
        let run_retention = self.rt.lock().unwrap().last_retention.map(|l| at - l > Duration::hours(12)).unwrap_or(true);
        if run_retention {
            let _ = self.db(|c| ingest::apply_retention(c, &p).map(|_| ()));
            let _ = self.db(|c| {
                c.execute_batch("PRAGMA optimize; PRAGMA wal_checkpoint(TRUNCATE);")?;
                Ok(())
            });
            self.rt.lock().unwrap().last_retention = Some(at);
        }
        rep
    }

    // ------------------------------------------------------------------ data management

    pub fn export_json(&self, dest: &Path) -> Result<()> {
        let v = self.db(|c| {
            let dump = |sql: &str| -> Result<Vec<serde_json::Value>> {
                let mut st = c.prepare(sql)?;
                let cols: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
                let rows = st.query_map([], |r| {
                    let mut m = serde_json::Map::new();
                    for (i, col) in cols.iter().enumerate() {
                        let v: rusqlite::types::Value = r.get(i)?;
                        m.insert(
                            col.clone(),
                            match v {
                                rusqlite::types::Value::Null => serde_json::Value::Null,
                                rusqlite::types::Value::Integer(i) => i.into(),
                                rusqlite::types::Value::Real(f) => f.into(),
                                rusqlite::types::Value::Text(t) => t.into(),
                                rusqlite::types::Value::Blob(_) => serde_json::Value::Null,
                            },
                        );
                    }
                    Ok(serde_json::Value::Object(m))
                })?;
                Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
            };
            Ok(serde_json::json!({
                "exported_at": ts(&now()),
                "format": "kept-export-v1",
                "note": "Contains your locally stored Kept data. OAuth tokens and API keys are not included.",
                "preferences": prefs::load(c)?,
                "accounts": dump("SELECT id, provider, display_name, email, status, connected_at, last_success_at FROM accounts")?,
                "tasks": dump("SELECT * FROM tasks")?,
                "task_sources": dump("SELECT * FROM task_sources")?,
                "task_deadlines": dump("SELECT * FROM task_deadlines")?,
                "commitments": dump("SELECT * FROM commitments")?,
                "meetings": dump("SELECT * FROM meetings")?,
                "source_items": dump("SELECT id, account_id, provider, kind, thread_key, sender_name, sender_email, subject, snippet, sent_at, is_from_me, url FROM source_items")?,
                "notifications": dump("SELECT * FROM notifications")?,
                "audit_events": dump("SELECT * FROM audit_events")?,
                "ai_runs": dump("SELECT * FROM ai_runs")?,
                "documents": dump("SELECT * FROM documents")?,
                "external_tasks": dump("SELECT * FROM external_tasks")?,
            }))
        })?;
        std::fs::write(dest, serde_json::to_vec_pretty(&v)?)?;
        Ok(())
    }

    pub fn backup(&self, dest: &Path) -> Result<()> {
        self.db(|c| db::backup_to(c, dest))
    }

    /// Delete everything stored locally (and the tokens in the credential store).
    pub fn delete_all_data(&self) -> Result<()> {
        for a in self.accounts()? {
            let _ = self.secrets.delete(&secrets::token_key_name(&a.id));
        }
        for prov in ["anthropic", "openai", "openai_compatible"] {
            let _ = self.secrets.delete(&secrets::ai_key_name(prov));
        }
        let _ = self.secrets.delete(&secrets::client_secret_key_name("google"));
        self.db(|c| {
            c.execute_batch(
                "DELETE FROM notifications; DELETE FROM audit_events; DELETE FROM commitments; DELETE FROM task_dependencies;
                 DELETE FROM task_sources; DELETE FROM task_deadlines; DELETE FROM tasks; DELETE FROM projects; DELETE FROM meeting_attendees;
                 DELETE FROM meetings; DELETE FROM source_items; DELETE FROM documents; DELETE FROM external_tasks; DELETE FROM sync_cursors; DELETE FROM accounts; DELETE FROM ai_runs;
                 DELETE FROM external_actions; DELETE FROM preferences; VACUUM;",
            )?;
            Ok(())
        })
    }

    pub fn storage_info(&self) -> Result<serde_json::Value> {
        let size = |p: PathBuf| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let db = size(self.data_dir.join("kept.db")) + size(self.data_dir.join("kept.db-wal"));
        let counts = self.db(|c| {
            let q = |t: &str| -> i64 { c.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0)).unwrap_or(0) };
            Ok(serde_json::json!({
                "tasks": q("tasks"), "source_items": q("source_items"), "meetings": q("meetings"),
                "notifications": q("notifications"), "audit_events": q("audit_events"), "ai_runs": q("ai_runs"),
            }))
        })?;
        let ai = self.db(|c| {
            let since = ts(&(now() - Duration::days(30)));
            let (n, chars, fails): (i64, i64, i64) = c.query_row(
                "SELECT COUNT(*), COALESCE(SUM(request_chars),0), COALESCE(SUM(CASE WHEN ok=0 THEN 1 ELSE 0 END),0) FROM ai_runs WHERE created_at > ?1",
                [since],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            Ok(serde_json::json!({"requests_30d": n, "chars_sent_30d": chars, "failures_30d": fails}))
        })?;
        Ok(serde_json::json!({ "data_dir": self.data_dir.display().to_string(), "database_bytes": db, "counts": counts, "ai": ai }))
    }
}
