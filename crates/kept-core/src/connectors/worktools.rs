//! Task tools: Jira Cloud, Asana, Todoist and Linear.
//!
//! Each tool's official *personal API token* is used (stored in Windows Credential Manager;
//! you can revoke it in the tool at any time). These providers' browser sign-in flows require
//! a server-held client secret, which a desktop app cannot keep secret, so tokens are the
//! secure option for a local app.
//!
//! Kept reads the tasks assigned to you and mirrors them as local tasks, so commitments found
//! in email/chat are de-duplicated against what's already tracked, and a task closed in the
//! tool is closed in Kept with that as evidence. Writing back (Todoist) only happens after you
//! confirm in the app.

use super::http::{base, request_json, Auth};
use super::{ConnectorError, SyncOutput};
use crate::models::IncomingExternalTask;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use reqwest::Method;
use serde_json::{json, Value};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct TokenSecret {
    pub token: String,
    #[serde(default)]
    pub email: Option<String>,
    /// Jira: https://your-site.atlassian.net
    #[serde(default)]
    pub site: Option<String>,
}

pub struct Identity {
    pub label: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

fn date_only(s: &str) -> Option<DateTime<Utc>> {
    // A date-only deadline is stored as 00:00 UTC; ingest converts it to end of *your* workday.
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc())
}

fn dt(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc)).or_else(|| {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok().map(|n| n.and_utc())
    })
}

// ---------------------------------------------------------------- Jira Cloud

fn jira_site(sec: &TokenSecret) -> Result<String, ConnectorError> {
    let s = sec.site.clone().unwrap_or_default();
    let s = s.trim().trim_end_matches('/').to_string();
    let s = if s.starts_with("http") { s } else { format!("https://{s}") };
    let u = url::Url::parse(&s).map_err(|_| ConnectorError::Permanent("Jira site URL is not valid".into()))?;
    if u.scheme() != "https" && !cfg!(debug_assertions) {
        return Err(ConnectorError::Permanent("Jira site must use https".into()));
    }
    Ok(s)
}

pub async fn jira_whoami(http: &reqwest::Client, sec: &TokenSecret) -> Result<Identity, ConnectorError> {
    let site = jira_site(sec)?;
    let email = sec.email.clone().unwrap_or_default();
    let v = request_json(http, Method::GET, &format!("{site}/rest/api/3/myself"), Auth::Basic(&email, &sec.token), None, &[]).await?;
    Ok(Identity {
        label: format!("Jira · {}", site.trim_start_matches("https://")),
        email: v["emailAddress"].as_str().map(|s| s.to_lowercase()).or(Some(email.to_lowercase())),
        name: v["displayName"].as_str().map(|s| s.to_string()),
    })
}

pub fn parse_jira_issue(site: &str, i: &Value) -> Option<IncomingExternalTask> {
    let f = &i["fields"];
    let key = i["key"].as_str()?;
    let prio = match f["priority"]["name"].as_str().unwrap_or("") {
        "Highest" | "Blocker" | "Critical" => Some(1),
        "High" | "Major" => Some(2),
        "Low" | "Lowest" | "Minor" | "Trivial" => Some(4),
        _ => None,
    };
    Some(IncomingExternalTask {
        external_id: i["id"].as_str().unwrap_or(key).to_string(),
        key: Some(key.to_string()),
        title: f["summary"].as_str()?.to_string(),
        description: None,
        status: f["status"]["name"].as_str().map(|s| s.to_string()),
        is_done: f["status"]["statusCategory"]["key"].as_str() == Some("done"),
        due_at: f["duedate"].as_str().and_then(date_only),
        due_is_date_only: true,
        url: Some(format!("{site}/browse/{key}")),
        project: f["project"]["name"].as_str().map(|s| s.to_string()),
        priority: prio,
    })
}

pub async fn jira_sync(http: &reqwest::Client, sec: &TokenSecret, out: &mut SyncOutput) -> Result<(), ConnectorError> {
    let site = jira_site(sec)?;
    let email = sec.email.clone().unwrap_or_default();
    let mut tasks = vec![];
    let mut next: Option<String> = None;
    let mut complete = true;
    for page in 0.. {
        let mut body = json!({
            "jql": "assignee = currentUser() AND (statusCategory != Done OR resolved >= -14d) ORDER BY updated DESC",
            "fields": ["summary", "status", "duedate", "project", "priority"],
            "maxResults": 100
        });
        if let Some(t) = &next {
            body["nextPageToken"] = json!(t);
        }
        let v = request_json(http, Method::POST, &format!("{site}/rest/api/3/search/jql"), Auth::Basic(&email, &sec.token), Some(&body), &[]).await?;
        tasks.extend(v["issues"].as_array().cloned().unwrap_or_default().iter().filter_map(|i| parse_jira_issue(&site, i)));
        next = v["nextPageToken"].as_str().map(|s| s.to_string());
        if next.is_none() {
            break;
        }
        if page >= 4 {
            complete = false;
            break;
        }
    }
    out.external_tasks = Some(tasks);
    if !complete {
        out.notes.push("More than 500 Jira issues assigned; only the 500 most recently updated are tracked".into());
        out.snapshot_incomplete = true;
    }
    Ok(())
}

// ---------------------------------------------------------------- Asana

fn asana() -> String {
    base("KEPT_TEST_ASANA_BASE", "https://app.asana.com/api/1.0")
}

pub async fn asana_whoami(http: &reqwest::Client, sec: &TokenSecret) -> Result<Identity, ConnectorError> {
    let v = request_json(http, Method::GET, &format!("{}/users/me?opt_fields=name,email,workspaces.name", asana()), Auth::Bearer(&sec.token), None, &[]).await?;
    let d = &v["data"];
    Ok(Identity {
        label: format!("Asana · {}", d["email"].as_str().unwrap_or("account")),
        email: d["email"].as_str().map(|s| s.to_lowercase()),
        name: d["name"].as_str().map(|s| s.to_string()),
    })
}

pub fn parse_asana_task(t: &Value) -> Option<IncomingExternalTask> {
    let gid = t["gid"].as_str()?;
    let (due, date_only_flag) = match (t["due_at"].as_str(), t["due_on"].as_str()) {
        (Some(a), _) => (dt(a), false),
        (None, Some(d)) => (date_only(d), true),
        _ => (None, false),
    };
    Some(IncomingExternalTask {
        external_id: gid.to_string(),
        key: None,
        title: t["name"].as_str().filter(|s| !s.is_empty())?.to_string(),
        description: t["notes"].as_str().filter(|s| !s.is_empty()).map(|s| crate::text::truncate(s, 1000)),
        status: Some(if t["completed"].as_bool() == Some(true) { "Completed".into() } else { "Open".into() }),
        is_done: t["completed"].as_bool() == Some(true),
        due_at: due,
        due_is_date_only: date_only_flag,
        url: t["permalink_url"].as_str().map(|s| s.to_string()),
        project: t["projects"][0]["name"].as_str().map(|s| s.to_string()),
        priority: None,
    })
}

pub async fn asana_sync(http: &reqwest::Client, sec: &TokenSecret, out: &mut SyncOutput) -> Result<(), ConnectorError> {
    let me = request_json(http, Method::GET, &format!("{}/users/me?opt_fields=workspaces", asana()), Auth::Bearer(&sec.token), None, &[]).await?;
    let since = (Utc::now() - Duration::days(14)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut tasks = vec![];
    for ws in me["data"]["workspaces"].as_array().cloned().unwrap_or_default().iter().take(10) {
        let Some(wid) = ws["gid"].as_str() else { continue };
        let mut offset: Option<String> = None;
        for _ in 0..5 {
            let mut url = format!(
                "{}/tasks?assignee=me&workspace={wid}&completed_since={}&limit=100&opt_fields=name,notes,completed,due_on,due_at,permalink_url,projects.name",
                asana(),
                urlencoding::encode(&since)
            );
            if let Some(o) = &offset {
                url.push_str(&format!("&offset={o}"));
            }
            let v = request_json(http, Method::GET, &url, Auth::Bearer(&sec.token), None, &[]).await?;
            tasks.extend(v["data"].as_array().cloned().unwrap_or_default().iter().filter_map(parse_asana_task));
            offset = v["next_page"]["offset"].as_str().map(|s| s.to_string());
            if offset.is_none() {
                break;
            }
        }
    }
    out.external_tasks = Some(tasks);
    Ok(())
}

// ---------------------------------------------------------------- Todoist (API v1)

fn todoist() -> String {
    base("KEPT_TEST_TODOIST_BASE", "https://api.todoist.com/api/v1")
}

pub async fn todoist_whoami(http: &reqwest::Client, sec: &TokenSecret) -> Result<Identity, ConnectorError> {
    let v = request_json(http, Method::GET, &format!("{}/user", todoist()), Auth::Bearer(&sec.token), None, &[]).await?;
    Ok(Identity {
        label: format!("Todoist · {}", v["email"].as_str().unwrap_or("account")),
        email: v["email"].as_str().map(|s| s.to_lowercase()),
        name: v["full_name"].as_str().map(|s| s.to_string()),
    })
}

pub fn parse_todoist_task(t: &Value, projects: &std::collections::HashMap<String, String>) -> Option<IncomingExternalTask> {
    let id = t["id"].as_str().map(|s| s.to_string()).or_else(|| t["id"].as_i64().map(|i| i.to_string()))?;
    let due = &t["due"];
    let (due_at, date_only_flag) = match due["datetime"].as_str().or(due["date"].as_str()) {
        Some(s) if s.len() == 10 => (date_only(s), true),
        Some(s) => (dt(s), false),
        None => (None, false),
    };
    let pid = t["project_id"].as_str().map(|s| s.to_string()).unwrap_or_default();
    Some(IncomingExternalTask {
        external_id: id.clone(),
        key: None,
        title: t["content"].as_str()?.to_string(),
        description: t["description"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string()),
        status: Some(if t["checked"].as_bool() == Some(true) { "Completed".into() } else { "Open".into() }),
        is_done: t["checked"].as_bool() == Some(true) || t["is_completed"].as_bool() == Some(true),
        due_at,
        due_is_date_only: date_only_flag,
        url: Some(format!("https://app.todoist.com/app/task/{id}")),
        project: projects.get(&pid).cloned(),
        // Todoist: 4 = urgent (p1) … 1 = normal (p4)
        priority: t["priority"].as_i64().map(|p| (5 - p).clamp(1, 4)),
    })
}

pub async fn todoist_sync(http: &reqwest::Client, sec: &TokenSecret, out: &mut SyncOutput) -> Result<(), ConnectorError> {
    let mut projects = std::collections::HashMap::new();
    if let Ok(p) = request_json(http, Method::GET, &format!("{}/projects?limit=200", todoist()), Auth::Bearer(&sec.token), None, &[]).await {
        for x in p["results"].as_array().cloned().unwrap_or_default() {
            if let (Some(id), Some(n)) = (x["id"].as_str(), x["name"].as_str()) {
                projects.insert(id.to_string(), n.to_string());
            }
        }
    }
    let mut tasks = vec![];
    let mut cursor: Option<String> = None;
    let mut complete = true;
    for i in 0.. {
        let mut url = format!("{}/tasks?limit=200", todoist());
        if let Some(c) = &cursor {
            url.push_str(&format!("&cursor={}", urlencoding::encode(c)));
        }
        let v = request_json(http, Method::GET, &url, Auth::Bearer(&sec.token), None, &[]).await?;
        tasks.extend(v["results"].as_array().cloned().unwrap_or_default().iter().filter_map(|t| parse_todoist_task(t, &projects)));
        cursor = v["next_cursor"].as_str().map(|s| s.to_string());
        if cursor.is_none() {
            break;
        }
        if i >= 9 {
            complete = false;
            break;
        }
    }
    out.external_tasks = Some(tasks);
    if !complete {
        out.snapshot_incomplete = true;
    }
    Ok(())
}

/// Create a task in Todoist (only called after the user confirms in the app).
pub async fn todoist_create(http: &reqwest::Client, sec: &TokenSecret, content: &str, description: Option<&str>, due: Option<DateTime<Utc>>) -> Result<(String, Option<String>), ConnectorError> {
    let mut body = json!({"content": content});
    if let Some(d) = description {
        body["description"] = json!(d);
    }
    if let Some(d) = due {
        body["due_datetime"] = json!(d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    let v = request_json(http, Method::POST, &format!("{}/tasks", todoist()), Auth::Bearer(&sec.token), Some(&body), &[]).await?;
    let id = v["id"].as_str().map(|s| s.to_string()).or_else(|| v["id"].as_i64().map(|i| i.to_string())).ok_or_else(|| ConnectorError::Permanent("Todoist did not return a task id".into()))?;
    Ok((id.clone(), Some(format!("https://app.todoist.com/app/task/{id}"))))
}

// ---------------------------------------------------------------- Linear (GraphQL)

fn linear() -> String {
    base("KEPT_TEST_LINEAR_BASE", "https://api.linear.app/graphql")
}

async fn linear_query(http: &reqwest::Client, sec: &TokenSecret, query: &str, vars: Value) -> Result<Value, ConnectorError> {
    let v = request_json(http, Method::POST, &linear(), Auth::Raw(&sec.token), Some(&json!({"query": query, "variables": vars})), &[]).await?;
    if let Some(errs) = v["errors"].as_array() {
        let msg = errs.first().and_then(|e| e["message"].as_str()).unwrap_or("Linear API error");
        if msg.to_lowercase().contains("authentication") {
            return Err(ConnectorError::Auth);
        }
        return Err(ConnectorError::Permanent(msg.to_string()));
    }
    Ok(v["data"].clone())
}

pub async fn linear_whoami(http: &reqwest::Client, sec: &TokenSecret) -> Result<Identity, ConnectorError> {
    let d = linear_query(http, sec, "query { viewer { id name email } }", json!({})).await?;
    Ok(Identity {
        label: format!("Linear · {}", d["viewer"]["email"].as_str().unwrap_or("account")),
        email: d["viewer"]["email"].as_str().map(|s| s.to_lowercase()),
        name: d["viewer"]["name"].as_str().map(|s| s.to_string()),
    })
}

pub fn parse_linear_issue(i: &Value) -> Option<IncomingExternalTask> {
    let state_type = i["state"]["type"].as_str().unwrap_or("");
    if state_type == "canceled" {
        return None;
    }
    Some(IncomingExternalTask {
        external_id: i["id"].as_str()?.to_string(),
        key: i["identifier"].as_str().map(|s| s.to_string()),
        title: i["title"].as_str()?.to_string(),
        description: None,
        status: i["state"]["name"].as_str().map(|s| s.to_string()),
        is_done: state_type == "completed",
        due_at: i["dueDate"].as_str().and_then(date_only),
        due_is_date_only: true,
        url: i["url"].as_str().map(|s| s.to_string()),
        project: i["project"]["name"].as_str().or(i["team"]["name"].as_str()).map(|s| s.to_string()),
        // Linear: 1 urgent, 2 high, 3 medium, 4 low, 0 none
        priority: i["priority"].as_i64().filter(|p| *p > 0).map(|p| p.clamp(1, 4)),
    })
}

pub async fn linear_sync(http: &reqwest::Client, sec: &TokenSecret, out: &mut SyncOutput) -> Result<(), ConnectorError> {
    let since = (Utc::now() - Duration::days(14)).to_rfc3339();
    let q = "query($since: DateTimeOrDuration!) { viewer { assignedIssues(first: 200, filter: { or: [ { completedAt: { null: true } }, { completedAt: { gt: $since } } ] }) {
        nodes { id identifier title url dueDate priority state { name type } project { name } team { name } } } } }";
    let d = linear_query(http, sec, q, json!({"since": since})).await?;
    let tasks = d["viewer"]["assignedIssues"]["nodes"].as_array().cloned().unwrap_or_default().iter().filter_map(parse_linear_issue).collect();
    out.external_tasks = Some(tasks);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_tool() {
        let j = json!({"id": "10001", "key": "OPS-12", "fields": {"summary": "Migrate billing DB", "duedate": "2026-10-21", "status": {"name": "In Progress", "statusCategory": {"key": "indeterminate"}}, "project": {"name": "Ops"}, "priority": {"name": "High"}}});
        let t = parse_jira_issue("https://acme.atlassian.net", &j).unwrap();
        assert_eq!(t.url.as_deref(), Some("https://acme.atlassian.net/browse/OPS-12"));
        assert_eq!(t.priority, Some(2));
        assert!(!t.is_done && t.due_is_date_only);

        let a = json!({"gid": "77", "name": "Draft launch email", "completed": true, "due_at": "2026-10-20T12:00:00.000Z", "permalink_url": "https://app.asana.com/0/1/77", "projects": [{"name": "Launch"}]});
        let t = parse_asana_task(&a).unwrap();
        assert!(t.is_done);
        assert_eq!(t.project.as_deref(), Some("Launch"));

        let mut projects = std::collections::HashMap::new();
        projects.insert("p1".to_string(), "Inbox".to_string());
        let td = json!({"id": "abc", "content": "Book flights", "project_id": "p1", "priority": 4, "due": {"date": "2026-10-22"}, "checked": false});
        let t = parse_todoist_task(&td, &projects).unwrap();
        assert_eq!(t.priority, Some(1));
        assert_eq!(t.project.as_deref(), Some("Inbox"));
        assert_eq!(t.due_at.unwrap().format("%Y-%m-%d").to_string(), "2026-10-22");

        let l = json!({"id": "u1", "identifier": "ENG-4", "title": "Fix login bug", "url": "https://linear.app/x/issue/ENG-4", "dueDate": "2026-10-25", "priority": 1, "state": {"name": "Done", "type": "completed"}, "team": {"name": "Eng"}});
        let t = parse_linear_issue(&l).unwrap();
        assert!(t.is_done);
        assert_eq!(t.key.as_deref(), Some("ENG-4"));
        assert!(parse_linear_issue(&json!({"id": "c", "title": "x", "state": {"type": "canceled"}})).is_none());
    }
}
