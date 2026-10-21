//! Google connector: Gmail API (messages + history) and Google Calendar API v3. Read-only scopes.

use super::http::{get_json, html_to_text};
use super::oauth::ProviderEndpoints;
use super::{ConnectorError, SyncOutput};
use crate::models::{Attendee, IncomingEvent, IncomingMessage};
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::Value;

pub const SCOPES: &[&str] = &[
    "openid",
    "email",
    "https://www.googleapis.com/auth/gmail.readonly",
    "https://www.googleapis.com/auth/calendar.readonly",
];
/// API base URLs. Debug builds allow overriding them so the sync engine can be tested against a mock server.
fn base(var: &str, default: &str) -> String {
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var(var) {
        return v;
    }
    let _ = var;
    default.to_string()
}
fn gmail() -> String {
    base("KEPT_TEST_GMAIL_BASE", "https://gmail.googleapis.com/gmail/v1/users/me")
}
fn cal() -> String {
    base("KEPT_TEST_GCAL_BASE", "https://www.googleapis.com/calendar/v3/calendars/primary/events")
}
/// Cap per sync run, so one huge mailbox can't stall the agent or the AI budget.
const MAX_MESSAGES_PER_RUN: usize = 150;

pub fn endpoints(client_id: &str, client_secret: Option<String>) -> ProviderEndpoints {
    endpoints_with(client_id, client_secret, SCOPES)
}

pub fn endpoints_with(client_id: &str, client_secret: Option<String>, scopes: &[&str]) -> ProviderEndpoints {
    ProviderEndpoints {
        auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
        token_url: "https://oauth2.googleapis.com/token".into(),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        client_id: client_id.into(),
        client_secret,
        redirect_host: "127.0.0.1",
        extra_auth_params: vec![("access_type".into(), "offline".into()), ("prompt".into(), "consent".into())],
        scope_param: "scope",
        fixed_port: None,
    }
}

pub async fn whoami(http: &reqwest::Client, token: &str) -> Result<(String, Option<String>), ConnectorError> {
    let v = get_json(http, "https://openidconnect.googleapis.com/v1/userinfo", token, &[]).await?;
    let email = v["email"].as_str().unwrap_or_default().to_lowercase();
    Ok((email, v["name"].as_str().map(|s| s.to_string())))
}

pub async fn revoke(http: &reqwest::Client, token: &str) {
    let _ = http.post("https://oauth2.googleapis.com/revoke").form(&[("token", token)]).send().await;
}

fn header<'a>(headers: &'a [Value], name: &str) -> Option<&'a str> {
    headers.iter().find(|h| h["name"].as_str().map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false)).and_then(|h| h["value"].as_str())
}

fn decode(data: &str) -> String {
    URL_SAFE_NO_PAD
        .decode(data.trim_end_matches('='))
        .or_else(|_| URL_SAFE.decode(data))
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default()
}

fn find_body(part: &Value, mime: &str) -> Option<String> {
    if part["mimeType"].as_str() == Some(mime) {
        if let Some(d) = part["body"]["data"].as_str() {
            return Some(decode(d));
        }
    }
    part["parts"].as_array().and_then(|ps| ps.iter().find_map(|p| find_body(p, mime)))
}

fn split_addrs(s: Option<&str>) -> Vec<String> {
    s.map(|s| {
        // Split on commas not inside quotes.
        let mut out = vec![];
        let mut cur = String::new();
        let mut q = false;
        for c in s.chars() {
            match c {
                '"' => {
                    q = !q;
                    cur.push(c)
                }
                ',' if !q => {
                    if !cur.trim().is_empty() {
                        out.push(cur.trim().to_string());
                    }
                    cur.clear();
                }
                _ => cur.push(c),
            }
        }
        if !cur.trim().is_empty() {
            out.push(cur.trim().to_string());
        }
        out
    })
    .unwrap_or_default()
}

pub fn parse_message(v: &Value) -> Option<IncomingMessage> {
    let id = v["id"].as_str()?;
    let labels: Vec<&str> = v["labelIds"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
    if labels.iter().any(|l| matches!(*l, "DRAFT" | "SPAM" | "TRASH" | "CATEGORY_PROMOTIONS" | "CATEGORY_SOCIAL")) {
        return None;
    }
    let headers = v["payload"]["headers"].as_array().cloned().unwrap_or_default();
    let (sender_name, sender_email) = crate::text::parse_address(header(&headers, "From").unwrap_or(""));
    let body = find_body(&v["payload"], "text/plain").or_else(|| find_body(&v["payload"], "text/html").map(|h| html_to_text(&h))).unwrap_or_else(|| v["snippet"].as_str().unwrap_or("").to_string());
    let ms: i64 = v["internalDate"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0);
    let sent_at = DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_else(Utc::now);
    let thread = v["threadId"].as_str().unwrap_or(id);
    Some(IncomingMessage {
        external_id: id.into(),
        kind: "email".into(),
        thread_key: Some(format!("gmail:{thread}")),
        sender_name,
        sender_email: Some(sender_email),
        to: split_addrs(header(&headers, "To")),
        cc: split_addrs(header(&headers, "Cc")),
        subject: header(&headers, "Subject").map(|s| s.to_string()),
        body_text: body,
        url: Some(format!("https://mail.google.com/mail/u/0/#all/{thread}")),
        sent_at,
    })
}

async fn fetch_messages(http: &reqwest::Client, token: &str, ids: &[String], out: &mut SyncOutput) -> Result<(), ConnectorError> {
    for id in ids.iter().take(MAX_MESSAGES_PER_RUN) {
        match get_json(http, &format!("{}/messages/{id}?format=full", gmail()), token, &[]).await {
            Ok(v) => {
                if let Some(m) = parse_message(&v) {
                    out.messages.push(m);
                }
            }
            Err(ConnectorError::CursorExpired) => continue, // deleted meanwhile
            Err(e) => return Err(e),
        }
    }
    if ids.len() > MAX_MESSAGES_PER_RUN {
        out.notes.push(format!("{} more messages will be fetched on the next sync", ids.len() - MAX_MESSAGES_PER_RUN));
    }
    Ok(())
}

pub async fn sync_mail(http: &reqwest::Client, token: &str, cursor: Option<&str>, initial_days: u32, out: &mut SyncOutput) -> Result<(), ConnectorError> {
    if let Some(history_id) = cursor {
        let mut ids: Vec<String> = vec![];
        let mut page: Option<String> = None;
        let mut latest = history_id.to_string();
        loop {
            let mut url = format!("{}/history?startHistoryId={history_id}&historyTypes=messageAdded&maxResults=200", gmail());
            if let Some(p) = &page {
                url.push_str(&format!("&pageToken={p}"));
            }
            match get_json(http, &url, token, &[]).await {
                Ok(v) => {
                    if let Some(h) = v["historyId"].as_str() {
                        latest = h.to_string();
                    }
                    for h in v["history"].as_array().cloned().unwrap_or_default() {
                        for m in h["messagesAdded"].as_array().cloned().unwrap_or_default() {
                            if let Some(id) = m["message"]["id"].as_str() {
                                if !ids.iter().any(|x| x == id) {
                                    ids.push(id.to_string());
                                }
                            }
                        }
                    }
                    page = v["nextPageToken"].as_str().map(|s| s.to_string());
                    if page.is_none() || ids.len() > 500 {
                        break;
                    }
                }
                Err(ConnectorError::CursorExpired) => {
                    // History IDs expire after about a week: fall back to a short full sync.
                    out.reset.push("mail".into());
                    out.notes.push("Gmail history expired; re-synced the last 3 days".into());
                    return Box::pin(sync_mail(http, token, None, 3, out)).await;
                }
                Err(e) => return Err(e),
            }
        }
        // Only advance the cursor when we fetched everything we were told about.
        let complete = ids.len() <= MAX_MESSAGES_PER_RUN;
        fetch_messages(http, token, &ids, out).await?;
        if complete {
            out.cursors.push(("mail".into(), latest));
        }
        return Ok(());
    }
    // Initial sync: profile historyId first, so nothing between list and cursor is lost.
    let profile = get_json(http, &format!("{}/profile", gmail()), token, &[]).await?;
    let history_id = profile["historyId"].as_str().map(|s| s.to_string());
    let query = format!("newer_than:{initial_days}d -in:spam -in:trash -in:drafts -category:promotions -category:social -category:forums");
    let q = urlencoding::encode(&query);
    let mut ids = vec![];
    let mut page: Option<String> = None;
    loop {
        let mut url = format!("{}/messages?maxResults=100&q={q}", gmail());
        if let Some(p) = &page {
            url.push_str(&format!("&pageToken={p}"));
        }
        let v = get_json(http, &url, token, &[]).await?;
        for m in v["messages"].as_array().cloned().unwrap_or_default() {
            if let Some(id) = m["id"].as_str() {
                ids.push(id.to_string());
            }
        }
        page = v["nextPageToken"].as_str().map(|s| s.to_string());
        if page.is_none() || ids.len() >= MAX_MESSAGES_PER_RUN {
            break;
        }
    }
    fetch_messages(http, token, &ids, out).await?;
    if let Some(h) = history_id {
        out.cursors.push(("mail".into(), h));
    }
    Ok(())
}

fn parse_time(v: &Value) -> Option<(DateTime<Utc>, bool)> {
    if let Some(dt) = v["dateTime"].as_str() {
        return DateTime::parse_from_rfc3339(dt).ok().map(|d| (d.with_timezone(&Utc), false));
    }
    v["date"].as_str().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).map(|d| (d.and_hms_opt(0, 0, 0).unwrap().and_utc(), true))
}

pub fn parse_event(e: &Value) -> Option<IncomingEvent> {
    let id = e["id"].as_str()?;
    let status = e["status"].as_str().unwrap_or("confirmed").to_string();
    let (start, all_day) = parse_time(&e["start"]).unwrap_or((Utc::now(), false));
    let (end, _) = parse_time(&e["end"]).unwrap_or((start + Duration::minutes(30), false));
    let attendees: Vec<Attendee> = e["attendees"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|x| !x["resource"].as_bool().unwrap_or(false))
                .filter_map(|x| {
                    Some(Attendee {
                        email: x["email"].as_str()?.to_lowercase(),
                        name: x["displayName"].as_str().map(|s| s.to_string()),
                        response: x["responseStatus"].as_str().map(|s| s.to_string()),
                        optional: x["optional"].as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let my_response = e["attendees"].as_array().and_then(|a| a.iter().find(|x| x["self"].as_bool().unwrap_or(false))).and_then(|x| x["responseStatus"].as_str()).map(|s| s.to_string());
    let join = e["hangoutLink"].as_str().map(|s| s.to_string()).or_else(|| {
        e["conferenceData"]["entryPoints"].as_array().and_then(|a| a.iter().find(|p| p["entryPointType"] == "video")).and_then(|p| p["uri"].as_str()).map(|s| s.to_string())
    });
    Some(IncomingEvent {
        external_id: id.into(),
        title: e["summary"].as_str().unwrap_or("(no title)").to_string(),
        description: e["description"].as_str().map(|d| if d.contains('<') { html_to_text(d) } else { d.to_string() }),
        location: e["location"].as_str().map(|s| s.to_string()),
        start_at: start,
        end_at: end,
        timezone: e["start"]["timeZone"].as_str().map(|s| s.to_string()),
        all_day,
        organizer_email: e["organizer"]["email"].as_str().map(|s| s.to_lowercase()),
        organizer_name: e["organizer"]["displayName"].as_str().map(|s| s.to_string()),
        attendees,
        join_url: join,
        url: e["htmlLink"].as_str().map(|s| s.to_string()),
        status,
        my_response,
    })
}

/// Calendar: re-read a bounded window each run (small, robust; catches edits and cancellations).
pub async fn sync_calendar(http: &reqwest::Client, token: &str, out: &mut SyncOutput) -> Result<(), ConnectorError> {
    let from = (Utc::now() - Duration::days(2)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let to = (Utc::now() + Duration::days(30)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut page: Option<String> = None;
    for _ in 0..5 {
        let mut url = format!(
            "{}?singleEvents=true&orderBy=startTime&showDeleted=true&maxResults=250&timeMin={}&timeMax={}",
            cal(),
            urlencoding::encode(&from),
            urlencoding::encode(&to)
        );
        if let Some(p) = &page {
            url.push_str(&format!("&pageToken={p}"));
        }
        let v = get_json(http, &url, token, &[]).await?;
        for e in v["items"].as_array().cloned().unwrap_or_default() {
            if let Some(ev) = parse_event(&e) {
                out.events.push(ev);
            }
        }
        page = v["nextPageToken"].as_str().map(|s| s.to_string());
        if page.is_none() {
            break;
        }
    }
    out.cursors.push(("calendar".into(), Utc::now().to_rfc3339()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_gmail_message() {
        let body = URL_SAFE_NO_PAD.encode("Hi,\nCould you send me the deck by Friday?\n\nOn Mon, Sara wrote:\n> old");
        let v = json!({
            "id": "m1", "threadId": "t1", "internalDate": "1760000000000", "labelIds": ["INBOX"],
            "payload": {"mimeType": "multipart/alternative", "headers": [
                {"name": "From", "value": "\"Khan, Sara\" <Sara@Acme.com>"},
                {"name": "To", "value": "me@corp.com, \"Lee, Bo\" <bo@corp.com>"},
                {"name": "Subject", "value": "Deck"}],
              "parts": [{"mimeType": "text/plain", "body": {"data": body}}]}
        });
        let m = parse_message(&v).unwrap();
        assert_eq!(m.sender_email.as_deref(), Some("sara@acme.com"));
        assert_eq!(m.to.len(), 2);
        assert!(m.body_text.contains("send me the deck"));
        assert_eq!(m.thread_key.as_deref(), Some("gmail:t1"));
    }

    #[test]
    fn skips_drafts_and_promotions() {
        let v = json!({"id": "m", "labelIds": ["DRAFT"], "payload": {"headers": []}});
        assert!(parse_message(&v).is_none());
    }

    #[test]
    fn parses_calendar_event() {
        let e = json!({"id": "e1", "summary": "Project Review", "status": "confirmed",
            "start": {"dateTime": "2026-10-19T10:00:00+05:00", "timeZone": "Asia/Karachi"},
            "end": {"dateTime": "2026-10-19T11:00:00+05:00"},
            "attendees": [{"email": "me@corp.com", "self": true, "responseStatus": "accepted"}, {"email": "Lead@corp.com", "displayName": "Lead"}],
            "hangoutLink": "https://meet.google.com/abc"});
        let ev = parse_event(&e).unwrap();
        assert_eq!(ev.start_at.to_rfc3339(), "2026-10-19T05:00:00+00:00");
        assert_eq!(ev.my_response.as_deref(), Some("accepted"));
        assert_eq!(ev.attendees[1].email, "lead@corp.com");
    }
}
