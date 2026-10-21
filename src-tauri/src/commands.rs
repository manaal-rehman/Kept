//! IPC commands exposed to the dashboard. Every command validates its input; the webview
//! never receives secrets (keys and tokens are write-only from the UI's point of view).

use crate::AppState;
use kept_core::models::status;
use kept_core::tasks::{self, NewTask, TaskFilter, TaskPatch};
use kept_core::{followups, ingest, meetings, notify, planner, prefs::Preferences, secrets};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

type R<T> = Result<T, String>;

fn e<E: std::fmt::Display>(err: E) -> String {
    format!("{err:#}")
}

/// Tell the window (if open) that data changed.
pub fn changed(app: &AppHandle) {
    let _ = app.emit("kept://changed", ());
}

#[tauri::command]
pub fn get_dashboard(s: State<AppState>) -> R<planner::Dashboard> {
    let p = s.agent.prefs().map_err(e)?;
    s.agent.db(|c| planner::dashboard(c, &p)).map_err(e)
}

#[tauri::command]
pub fn get_day_plan(s: State<AppState>) -> R<planner::DayPlan> {
    let p = s.agent.prefs().map_err(e)?;
    s.agent.db(|c| planner::day_plan(c, &p, chrono::Utc::now())).map_err(e)
}

#[tauri::command]
pub fn list_tasks(s: State<AppState>, filter: TaskFilter) -> R<Vec<kept_core::models::Task>> {
    s.agent.db(|c| tasks::list(c, &filter)).map_err(e)
}

#[tauri::command]
pub fn get_task(s: State<AppState>, id: String) -> R<kept_core::models::TaskDetail> {
    s.agent.db(|c| tasks::detail(c, &id)).map_err(e)
}

#[tauri::command]
pub fn create_task(app: AppHandle, s: State<AppState>, task: NewTask) -> R<String> {
    let mut t = task;
    t.origin = "manual".into();
    t.confidence = 1.0;
    t.needs_confirmation = false;
    if t.status.is_empty() {
        t.status = status::READY.into();
    }
    if t.due_at.is_some() {
        t.due_kind = "confirmed".into();
    }
    let p = s.agent.prefs().map_err(e)?;
    let id = s.agent.db(|c| {
        let id = tasks::create(c, &t, "user")?;
        if t.priority.is_none() {
            planner::reprioritize_one(c, &p, &id)?;
        }
        Ok(id)
    }).map_err(e)?;
    changed(&app);
    Ok(id)
}

#[tauri::command]
pub fn update_task(app: AppHandle, s: State<AppState>, id: String, patch: TaskPatch) -> R<()> {
    let p = s.agent.prefs().map_err(e)?;
    s.agent.db(|c| {
        tasks::update(c, &id, &patch, "user")?;
        planner::reprioritize_one(c, &p, &id)
    }).map_err(e)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn set_task_status(app: AppHandle, s: State<AppState>, id: String, new_status: String, reason: Option<String>, evidence: Option<String>) -> R<()> {
    s.agent.db(|c| tasks::set_status(c, &id, &new_status, "user", reason.as_deref().unwrap_or(""), evidence.as_deref())).map_err(e)?;
    changed(&app);
    Ok(())
}

#[derive(Deserialize)]
pub struct ReviewAction {
    pub action: String, // accept | reject | merge | waiting
    pub ids: Vec<String>,
    pub merge_into: Option<String>,
    pub reason: Option<String>,
}

#[tauri::command]
pub fn review_tasks(app: AppHandle, s: State<AppState>, req: ReviewAction) -> R<usize> {
    let n = s.agent.db(|c| {
        let mut n = 0;
        for id in &req.ids {
            match req.action.as_str() {
                "accept" => tasks::accept(c, id)?,
                "reject" => tasks::reject(c, id, req.reason.as_deref().unwrap_or(""))?,
                "waiting" => {
                    tasks::accept(c, id)?;
                    tasks::set_status(c, id, status::WAITING, "user", "waiting for someone", None)?
                }
                "merge" => {
                    let into = req.merge_into.as_deref().ok_or_else(|| anyhow::anyhow!("merge target required"))?;
                    tasks::merge(c, into, id, "user")?
                }
                _ => anyhow::bail!("unknown action"),
            }
            n += 1;
        }
        Ok(n)
    }).map_err(e)?;
    changed(&app);
    Ok(n)
}

#[tauri::command]
pub fn undo_change(app: AppHandle, s: State<AppState>, audit_id: String) -> R<()> {
    s.agent.db(|c| tasks::undo(c, &audit_id)).map_err(e)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_task(app: AppHandle, s: State<AppState>, id: String) -> R<()> {
    s.agent.db(|c| tasks::delete(c, &id)).map_err(e)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn add_dependency(app: AppHandle, s: State<AppState>, id: String, depends_on: String, remove: bool) -> R<()> {
    s.agent.db(|c| if remove { tasks::remove_dependency(c, &id, &depends_on) } else { tasks::add_dependency(c, &id, &depends_on, None) }).map_err(e)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn get_source(s: State<AppState>, id: String) -> R<Option<kept_core::models::SourceItem>> {
    s.agent.db(|c| ingest::get_item(c, &id)).map_err(e)
}

#[tauri::command]
pub fn list_meetings(s: State<AppState>, days: i64) -> R<Vec<kept_core::models::Meeting>> {
    let now = chrono::Utc::now();
    s.agent.db(|c| meetings::list_range(c, now - chrono::Duration::hours(12), now + chrono::Duration::days(days.clamp(1, 60)))).map_err(e)
}

#[tauri::command]
pub fn get_briefing(s: State<AppState>, id: String) -> R<meetings::Briefing> {
    let p = s.agent.prefs().map_err(e)?;
    s.agent.db(|c| meetings::briefing(c, &p, &id)).map_err(e)
}

#[tauri::command]
pub async fn ai_briefing(s: State<'_, AppState>, id: String) -> R<String> {
    s.agent.ai_meeting_summary(&id).await.map_err(e)
}

#[tauri::command]
pub fn list_followups(s: State<AppState>) -> R<followups::Followups> {
    s.agent.db(followups::list).map_err(e)
}

#[tauri::command]
pub fn set_commitment(app: AppHandle, s: State<AppState>, id: String, new_status: String) -> R<()> {
    s.agent.db(|c| followups::set_commitment_status(c, &id, &new_status)).map_err(e)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn list_notifications(s: State<AppState>, scope: String) -> R<Vec<kept_core::models::Notification>> {
    s.agent.db(|c| notify::list(c, &scope, 300)).map_err(e)
}

#[tauri::command]
pub fn notification_action(app: AppHandle, s: State<AppState>, id: Option<String>, action: String, minutes: Option<i64>) -> R<()> {
    s.agent.db(|c| match (action.as_str(), id.as_deref()) {
        ("snooze", Some(i)) => notify::snooze(c, i, minutes.unwrap_or(60)),
        ("dismiss", Some(i)) => notify::dismiss(c, i),
        ("read", i) => notify::mark_read(c, i),
        _ => anyhow::bail!("unknown action"),
    }).map_err(e)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn get_rules(s: State<AppState>) -> R<Vec<kept_core::models::NotificationRule>> {
    s.agent.db(notify::rules).map_err(e)
}

#[tauri::command]
pub fn save_rule(s: State<AppState>, rule: kept_core::models::NotificationRule) -> R<()> {
    s.agent.db(|c| notify::save_rule(c, &rule)).map_err(e)
}

#[tauri::command]
pub fn test_notification(app: AppHandle) -> R<()> {
    app.notification().builder().title("Kept notifications work").body("This is how reminders will appear.").show().map_err(e)
}

#[tauri::command]
pub fn get_prefs(s: State<AppState>) -> R<Preferences> {
    s.agent.prefs().map_err(e)
}

#[tauri::command]
pub fn save_prefs(app: AppHandle, s: State<AppState>, prefs: Preferences) -> R<Preferences> {
    let old = s.agent.prefs().map_err(e)?;
    s.agent.save_prefs(&prefs).map_err(e)?;
    if old.autostart != prefs.autostart {
        let al = app.autolaunch();
        let r = if prefs.autostart { al.enable() } else { al.disable() };
        r.map_err(|err| format!("Saved, but could not change startup setting: {err}"))?;
    }
    if old.monitoring_paused != prefs.monitoring_paused {
        crate::refresh_tray(&app);
    }
    s.wake.notify_one();
    changed(&app);
    s.agent.prefs().map_err(e)
}

#[derive(Serialize)]
pub struct IntegrationsView {
    pub providers: Vec<kept_core::connectors::ProviderInfo>,
    pub accounts: Vec<kept_core::models::Account>,
    pub google_configured: bool,
    pub google_secret_saved: bool,
    pub microsoft_configured: bool,
    pub slack_configured: bool,
    pub keychain_ok: bool,
}

#[tauri::command]
pub fn get_integrations(s: State<AppState>) -> R<IntegrationsView> {
    let p = s.agent.prefs().map_err(e)?;
    Ok(IntegrationsView {
        providers: kept_core::connectors::catalog(),
        accounts: s.agent.accounts().map_err(e)?,
        google_configured: !p.google_client_id.is_empty() || option_env!("KEPT_GOOGLE_CLIENT_ID").is_some(),
        google_secret_saved: s.agent.secrets.get(&secrets::client_secret_key_name("google")).ok().flatten().is_some() || option_env!("KEPT_GOOGLE_CLIENT_SECRET").is_some(),
        microsoft_configured: !p.microsoft_client_id.is_empty() || option_env!("KEPT_MS_CLIENT_ID").is_some(),
        slack_configured: !p.slack_client_id.is_empty() || option_env!("KEPT_SLACK_CLIENT_ID").is_some(),
        keychain_ok: s.keychain_ok,
    })
}

/// Starts the browser sign-in and returns immediately; the result arrives as an event.
#[tauri::command]
pub fn connect_account(app: AppHandle, s: State<AppState>, provider: String) -> R<()> {
    if !matches!(provider.as_str(), "google" | "microsoft" | "gdrive" | "onedrive" | "teams" | "slack") {
        return Err("This service connects with an API token, not a browser sign-in.".into());
    }
    let agent = s.agent.clone();
    let wake = s.wake.clone();
    tauri::async_runtime::spawn(async move {
        let app2 = app.clone();
        let res = agent
            .connect(&provider, move |url| app2.opener().open_url(url, None::<&str>).map_err(|err| anyhow::anyhow!("could not open the browser: {err}")))
            .await;
        match res {
            Ok(acct) => {
                let _ = app.emit("kept://connect", json!({"ok": true, "provider": provider, "account": acct.display_name}));
                wake.notify_one();
            }
            Err(err) => {
                tracing::warn!("connect {provider} failed: {err:#}");
                let _ = app.emit("kept://connect", json!({"ok": false, "provider": provider, "error": format!("{err:#}")}));
            }
        }
        changed(&app);
    });
    Ok(())
}

#[tauri::command]
pub async fn sync_now(app: AppHandle, s: State<'_, AppState>, account_id: Option<String>) -> R<String> {
    let msg = match account_id {
        Some(id) => s.agent.sync_account(&id).await.map_err(e)?,
        None => {
            let mut out = vec![];
            for a in s.agent.accounts().map_err(e)? {
                if kept_core::agent::ONLINE_PROVIDERS.contains(&a.provider.as_str()) && a.status != "attention" {
                    match s.agent.sync_account(&a.id).await {
                        Ok(m) => out.push(format!("{}: {m}", a.display_name)),
                        Err(err) => out.push(format!("{}: {err}", a.display_name)),
                    }
                }
            }
            if out.is_empty() { "No online accounts to sync.".to_string() } else { out.join("\n") }
        }
    };
    let (n, st) = s.agent.process_pending(80).await.map_err(e)?;
    changed(&app);
    Ok(if n > 0 { format!("{msg}\nAnalysed {n} new items: {} tasks, {} for review, {} updated.", st.created, st.for_review, st.updated) } else { msg })
}

#[tauri::command]
pub async fn disconnect_account(app: AppHandle, s: State<'_, AppState>, id: String, delete_tasks: bool) -> R<()> {
    s.agent.disconnect(&id, delete_tasks).await.map_err(e)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn set_account_streams(s: State<AppState>, id: String, streams: Vec<String>) -> R<()> {
    let streams: Vec<String> = streams.into_iter().filter(|x| matches!(x.as_str(), "mail" | "calendar")).collect();
    s.agent.set_streams(&id, &streams).map_err(e)
}

/// Write-only secret setter. kind: ai:anthropic | ai:openai | ai:openai_compatible | client_secret:google
#[tauri::command]
pub fn set_secret(s: State<AppState>, kind: String, value: String) -> R<()> {
    let allowed = ["ai:anthropic", "ai:openai", "ai:openai_compatible", "client_secret:google"];
    if !allowed.contains(&kind.as_str()) {
        return Err("unknown secret".into());
    }
    let v = value.trim();
    if v.is_empty() {
        return s.agent.secrets.delete(&kind).map_err(e);
    }
    if v.len() > 8192 || v.chars().any(|c| c.is_control()) {
        return Err("That doesn't look like a valid key.".into());
    }
    s.agent.secrets.set(&kind, v).map_err(e)
}

#[tauri::command]
pub fn has_secret(s: State<AppState>, kind: String) -> R<bool> {
    Ok(s.agent.secrets.get(&kind).ok().flatten().is_some())
}

#[tauri::command]
pub async fn test_ai(s: State<'_, AppState>) -> R<String> {
    s.agent.ai_test().await.map_err(e)
}

#[tauri::command]
pub async fn import_files(app: AppHandle, s: State<'_, AppState>) -> R<Option<String>> {
    let picked = app
        .dialog()
        .file()
        .set_title("Import emails or calendar files")
        .add_filter("Email, calendar or Kept JSON", &["eml", "ics", "json"])
        .blocking_pick_files();
    let Some(files) = picked else { return Ok(None) };
    let paths: Vec<PathBuf> = files.into_iter().filter_map(|f| f.into_path().ok()).collect();
    let msg = s.agent.import_files(&paths).map_err(e)?;
    let (n, st) = s.agent.process_pending(200).await.map_err(e)?;
    changed(&app);
    s.wake.notify_one();
    Ok(Some(format!("{msg} Analysed {n}: {} new tasks, {} for review.", st.created, st.for_review)))
}

#[tauri::command]
pub async fn load_sample(app: AppHandle, s: State<'_, AppState>) -> R<String> {
    let msg = s.agent.load_sample().map_err(e)?;
    let (_n, st) = s.agent.process_pending(200).await.map_err(e)?;
    changed(&app);
    s.wake.notify_one();
    Ok(format!("{msg} {} new tasks, {} items to review, {} updates to existing tasks.", st.created, st.for_review, st.updated))
}

#[tauri::command]
pub fn export_data(s: State<AppState>, app: AppHandle) -> R<Option<String>> {
    let Some(path) = app.dialog().file().set_file_name("kept-export.json").add_filter("JSON", &["json"]).blocking_save_file() else { return Ok(None) };
    let p = path.into_path().map_err(e)?;
    s.agent.export_json(&p).map_err(e)?;
    Ok(Some(p.display().to_string()))
}

#[tauri::command]
pub fn backup_data(s: State<AppState>, app: AppHandle) -> R<Option<String>> {
    let name = format!("kept-backup-{}.db", chrono::Local::now().format("%Y%m%d-%H%M"));
    let Some(path) = app.dialog().file().set_file_name(&name).add_filter("SQLite database", &["db"]).blocking_save_file() else { return Ok(None) };
    let p = path.into_path().map_err(e)?;
    s.agent.backup(&p).map_err(e)?;
    Ok(Some(p.display().to_string()))
}

#[tauri::command]
pub fn delete_all_data(app: AppHandle, s: State<AppState>, confirm: String) -> R<()> {
    if confirm != "DELETE" {
        return Err("Type DELETE to confirm.".into());
    }
    s.agent.delete_all_data().map_err(e)?;
    let _ = app.autolaunch().disable();
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn storage_info(s: State<AppState>) -> R<Value> {
    s.agent.storage_info().map_err(e)
}

#[tauri::command]
pub fn audit_log(s: State<AppState>, limit: i64) -> R<Vec<Value>> {
    s.agent.db(|c| {
        let mut st = c.prepare("SELECT created_at, entity_type, action, from_value, to_value, actor, reason FROM audit_events ORDER BY created_at DESC LIMIT ?1")?;
        let v = st
            .query_map([limit.clamp(1, 1000)], |r| {
                Ok(json!({"at": r.get::<_, String>(0)?, "entity": r.get::<_, String>(1)?, "action": r.get::<_, String>(2)?, "from": r.get::<_, Option<String>>(3)?, "to": r.get::<_, Option<String>>(4)?, "actor": r.get::<_, String>(5)?, "reason": r.get::<_, Option<String>>(6)?}))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }).map_err(e)
}

#[tauri::command]
pub fn ai_usage(s: State<AppState>) -> R<Vec<Value>> {
    s.agent.db(|c| {
        let mut st = c.prepare("SELECT created_at, provider, model, purpose, request_chars, response_chars, items_returned, items_rejected, ok, error, duration_ms FROM ai_runs ORDER BY created_at DESC LIMIT 100")?;
        let v = st
            .query_map([], |r| {
                Ok(json!({"at": r.get::<_, String>(0)?, "provider": r.get::<_, String>(1)?, "model": r.get::<_, String>(2)?, "purpose": r.get::<_, String>(3)?, "request_chars": r.get::<_, i64>(4)?, "response_chars": r.get::<_, i64>(5)?, "items": r.get::<_, i64>(6)?, "rejected": r.get::<_, i64>(7)?, "ok": r.get::<_, bool>(8)?, "error": r.get::<_, Option<String>>(9)?, "ms": r.get::<_, i64>(10)?}))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }).map_err(e)
}

#[tauri::command]
pub fn open_external(app: AppHandle, url: String) -> R<()> {
    let u = url::Url::parse(&url).map_err(|_| "Invalid link".to_string())?;
    if !matches!(u.scheme(), "https" | "mailto") {
        return Err("Only https and mailto links can be opened.".into());
    }
    app.opener().open_url(u.as_str(), None::<&str>).map_err(e)
}

#[tauri::command]
pub fn open_data_folder(app: AppHandle, s: State<AppState>, which: String) -> R<()> {
    let dir = if which == "logs" { s.agent.data_dir.join("logs") } else { s.agent.data_dir.clone() };
    app.opener().open_path(dir.display().to_string(), None::<&str>).map_err(e)
}

#[tauri::command]
pub fn app_info(app: AppHandle, s: State<AppState>) -> R<Value> {
    Ok(json!({
        "version": app.package_info().version.to_string(),
        "data_dir": s.agent.data_dir.display().to_string(),
        "autostart": app.autolaunch().is_enabled().unwrap_or(false),
        "keychain_ok": s.keychain_ok,
        "os": std::env::consts::OS,
        "build_google": option_env!("KEPT_GOOGLE_CLIENT_ID").is_some(),
        "build_microsoft": option_env!("KEPT_MS_CLIENT_ID").is_some(),
    }))
}

#[tauri::command]
pub fn set_paused(app: AppHandle, s: State<AppState>, monitoring: Option<bool>, notifications_minutes: Option<i64>) -> R<()> {
    let mut p = s.agent.prefs().map_err(e)?;
    if let Some(m) = monitoring {
        p.monitoring_paused = m;
    }
    if let Some(mins) = notifications_minutes {
        p.notifications_paused_until = if mins <= 0 { None } else { Some(chrono::Utc::now() + chrono::Duration::minutes(mins.min(60 * 24 * 7))) };
    }
    s.agent.save_prefs(&p).map_err(e)?;
    crate::refresh_tray(&app);
    s.wake.notify_one();
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn hide_window(app: AppHandle) -> R<()> {
    if let Some(w) = app.get_webview_window("main") {
        w.close().map_err(e)?;
    }
    Ok(())
}

/// Re-check recent messages that produced no task (e.g. after adding an AI key).
#[tauri::command]
pub async fn reanalyse(app: AppHandle, s: State<'_, AppState>, days: i64) -> R<String> {
    let n = s.agent.reanalyse(days).map_err(e)?;
    if n == 0 {
        return Ok("No messages needed re-checking.".into());
    }
    let (_done, st) = s.agent.process_pending(500).await.map_err(e)?;
    changed(&app);
    Ok(format!("Re-checked {n} messages: {} new tasks, {} for review.", st.created, st.for_review))
}

/// Connect Jira / Asana / Todoist / Linear with a personal API token (verified before saving).
#[tauri::command]
pub async fn connect_token(app: AppHandle, s: State<'_, AppState>, provider: String, token: String, email: Option<String>, site: Option<String>) -> R<String> {
    if !matches!(provider.as_str(), "jira" | "asana" | "todoist" | "linear") {
        return Err("This service uses browser sign-in.".into());
    }
    let acct = s.agent.connect_with_token(&provider, &token, email.as_deref(), site.as_deref()).await.map_err(e)?;
    let msg = match s.agent.sync_account(&acct.id).await {
        Ok(m) => format!("Connected {}. {m}.", acct.display_name),
        Err(err) => format!("Connected {}, but the first sync failed: {err}", acct.display_name),
    };
    changed(&app);
    Ok(msg)
}

#[tauri::command]
pub fn related_documents(s: State<AppState>, task_id: String) -> R<Vec<kept_core::models::Document>> {
    s.agent
        .db(|c| {
            let t = tasks::get(c, &task_id)?;
            let q = format!("{} {} {}", t.title, t.expected_deliverable.unwrap_or_default(), t.project.unwrap_or_default());
            kept_core::documents::related(c, &q, 5)
        })
        .map_err(e)
}

/// Create the task in Todoist. The UI shows destination, content and effect and only calls
/// this after the user clicks Confirm.
#[tauri::command]
pub async fn push_to_todoist(app: AppHandle, s: State<'_, AppState>, task_id: String, confirmed: bool) -> R<String> {
    if !confirmed {
        return Err("Confirmation is required before Kept changes anything in Todoist.".into());
    }
    let r = s.agent.push_to_todoist(&task_id, None).await.map_err(e)?;
    changed(&app);
    Ok(r)
}

// ------------------------------------------------------------------ your own app connections

/// Validate one pasted value; returns a plain-language problem or None.
#[tauri::command]
pub fn check_connection_value(kind: String, value: String) -> Option<String> {
    kept_core::connsettings::check(&kind, &value)
}

/// Save the user's own app registration for google | microsoft | slack.
#[tauri::command]
pub fn save_connection(app: AppHandle, s: State<AppState>, provider: String, client_id: String, client_secret: Option<String>, tenant: Option<String>) -> R<()> {
    use kept_core::connsettings::check;
    let id = client_id.trim().to_string();
    let mut p = s.agent.prefs().map_err(e)?;
    match provider.as_str() {
        "google" => {
            if let Some(err) = check("google_client_id", &id) {
                return Err(err);
            }
            if let Some(sec) = client_secret.as_deref().map(str::trim).filter(|x| !x.is_empty()) {
                if let Some(err) = check("google_client_secret", sec) {
                    return Err(err);
                }
                s.agent.secrets.set(&secrets::client_secret_key_name("google"), sec).map_err(e)?;
            }
            p.google_client_id = id;
        }
        "microsoft" => {
            if let Some(err) = check("microsoft_client_id", &id) {
                return Err(err);
            }
            let t = tenant.unwrap_or_else(|| "common".into()).trim().to_string();
            let t = if t.is_empty() { "common".to_string() } else { t };
            if let Some(err) = check("microsoft_tenant", &t) {
                return Err(err);
            }
            p.microsoft_client_id = id;
            p.microsoft_tenant = t;
        }
        "slack" => {
            if let Some(err) = check("slack_client_id", &id) {
                return Err(err);
            }
            p.slack_client_id = id;
        }
        _ => return Err("unknown provider".into()),
    }
    s.agent.save_prefs(&p).map_err(e)?;
    changed(&app);
    Ok(())
}

#[tauri::command]
pub fn export_connection_settings(app: AppHandle, s: State<AppState>, include_google_secret: bool, organisation: Option<String>) -> R<Option<String>> {
    use kept_core::connsettings::*;
    let p = s.agent.prefs().map_err(e)?;
    let cs = ConnectionSettings {
        format: FORMAT.into(),
        organisation: organisation.filter(|o| !o.trim().is_empty()),
        google: (!p.google_client_id.is_empty()).then(|| GoogleApp {
            client_id: p.google_client_id.clone(),
            client_secret: if include_google_secret { s.agent.secrets.get(&secrets::client_secret_key_name("google")).ok().flatten() } else { None },
        }),
        microsoft: (!p.microsoft_client_id.is_empty()).then(|| MicrosoftApp { client_id: p.microsoft_client_id.clone(), tenant: p.microsoft_tenant.clone() }),
        slack: (!p.slack_client_id.is_empty()).then(|| SlackApp { client_id: p.slack_client_id.clone() }),
    };
    if cs.google.is_none() && cs.microsoft.is_none() && cs.slack.is_none() {
        return Err("Set up at least one connection first.".into());
    }
    let Some(path) = app.dialog().file().set_file_name("kept-connection-settings.json").add_filter("Kept connection settings", &["json"]).blocking_save_file() else { return Ok(None) };
    let path = path.into_path().map_err(e)?;
    std::fs::write(&path, serde_json::to_vec_pretty(&cs).map_err(e)?).map_err(e)?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
pub fn import_connection_settings(app: AppHandle, s: State<AppState>) -> R<Option<String>> {
    let Some(f) = app.dialog().file().set_title("Import connection settings").add_filter("Kept connection settings", &["json"]).blocking_pick_file() else { return Ok(None) };
    let path = f.into_path().map_err(e)?;
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 64 * 1024 {
        return Err("That file is too large to be a Kept connection settings file.".into());
    }
    let cs = kept_core::connsettings::parse(&std::fs::read_to_string(&path).map_err(e)?).map_err(e)?;
    let mut p = s.agent.prefs().map_err(e)?;
    let mut done = vec![];
    if let Some(g) = &cs.google {
        p.google_client_id = g.client_id.trim().into();
        if let Some(sec) = &g.client_secret {
            s.agent.secrets.set(&secrets::client_secret_key_name("google"), sec.trim()).map_err(e)?;
        }
        done.push("Google");
    }
    if let Some(m) = &cs.microsoft {
        p.microsoft_client_id = m.client_id.trim().into();
        p.microsoft_tenant = m.tenant.trim().into();
        done.push("Microsoft");
    }
    if let Some(sl) = &cs.slack {
        p.slack_client_id = sl.client_id.trim().into();
        done.push("Slack");
    }
    s.agent.save_prefs(&p).map_err(e)?;
    changed(&app);
    Ok(Some(format!(
        "Imported {} connection settings{}. You can now click Connect.",
        done.join(", "),
        cs.organisation.map(|o| format!(" from {o}")).unwrap_or_default()
    )))
}
