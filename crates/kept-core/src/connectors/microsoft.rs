//! Microsoft Graph connector: Outlook mail (inbox + sent, via delta queries) and calendarView.
//! Works for Microsoft 365 work/school accounts and personal Outlook.com accounts.
//! Some Exchange configurations (on-premises, restricted tenants, admin-consent policies)
//! may block Graph access — the connector surfaces that as "attention required".

use super::http::{get_json, html_to_text};
use super::oauth::ProviderEndpoints;
use super::{ConnectorError, SyncOutput};
use crate::models::{Attendee, IncomingEvent, IncomingMessage};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use serde_json::Value;

pub const SCOPES: &[&str] = &["offline_access", "User.Read", "Mail.Read", "Calendars.Read"];
const GRAPH: &str = "https://graph.microsoft.com/v1.0";
const MAX_PAGES: usize = 8;
const SELECT: &str = "id,conversationId,subject,from,toRecipients,ccRecipients,receivedDateTime,sentDateTime,body,webLink,isDraft";

pub fn endpoints(client_id: &str, tenant: &str) -> ProviderEndpoints {
    endpoints_with(client_id, tenant, SCOPES)
}

pub fn endpoints_with(client_id: &str, tenant: &str, scopes: &[&str]) -> ProviderEndpoints {
    let tenant = if tenant.trim().is_empty() { "common" } else { tenant.trim() };
    ProviderEndpoints {
        auth_url: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"),
        token_url: format!("https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"),
        scopes: scopes.iter().map(|s| s.to_string()).collect(),
        client_id: client_id.into(),
        client_secret: None,
        redirect_host: "localhost",
        extra_auth_params: vec![("prompt".into(), "select_account".into())],
        scope_param: "scope",
        fixed_port: None,
    }
}

pub async fn whoami(http: &reqwest::Client, token: &str) -> Result<(String, Option<String>), ConnectorError> {
    let v = get_json(http, &format!("{GRAPH}/me?$select=mail,userPrincipalName,displayName"), token, &[]).await?;
    let email = v["mail"].as_str().or(v["userPrincipalName"].as_str()).unwrap_or_default().to_lowercase();
    Ok((email, v["displayName"].as_str().map(|s| s.to_string())))
}

fn addr(v: &Value) -> Option<String> {
    let e = v["emailAddress"]["address"].as_str()?;
    Some(match v["emailAddress"]["name"].as_str() {
        Some(n) if !n.is_empty() && n != e => format!("{n} <{e}>"),
        _ => e.to_string(),
    })
}

pub fn parse_message(v: &Value) -> Option<IncomingMessage> {
    if v.get("@removed").is_some() || v["isDraft"].as_bool().unwrap_or(false) {
        return None;
    }
    let id = v["id"].as_str()?;
    let body = v["body"]["content"].as_str().unwrap_or("");
    let body = if v["body"]["contentType"].as_str() == Some("html") { html_to_text(body) } else { body.to_string() };
    let ts = v["sentDateTime"].as_str().or(v["receivedDateTime"].as_str()).and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&Utc)).unwrap_or_else(Utc::now);
    Some(IncomingMessage {
        external_id: id.into(),
        kind: "email".into(),
        thread_key: v["conversationId"].as_str().map(|c| format!("graph:{c}")),
        sender_name: v["from"]["emailAddress"]["name"].as_str().map(|s| s.to_string()),
        sender_email: v["from"]["emailAddress"]["address"].as_str().map(|s| s.to_lowercase()),
        to: v["toRecipients"].as_array().map(|a| a.iter().filter_map(addr).collect()).unwrap_or_default(),
        cc: v["ccRecipients"].as_array().map(|a| a.iter().filter_map(addr).collect()).unwrap_or_default(),
        subject: v["subject"].as_str().map(|s| s.to_string()),
        body_text: body,
        url: v["webLink"].as_str().map(|s| s.to_string()),
        sent_at: ts,
    })
}

/// Delta sync of one well-known folder ("inbox" or "sentitems"). Cursor = @odata.deltaLink.
pub async fn sync_folder(http: &reqwest::Client, token: &str, folder: &str, stream: &str, cursor: Option<&str>, initial_days: u32, out: &mut SyncOutput) -> Result<(), ConnectorError> {
    let headers = [("Prefer", "outlook.body-content-type=\"text\", odata.maxpagesize=50")];
    let mut url = match cursor {
        Some(c) => c.to_string(),
        None => {
            let since = (Utc::now() - Duration::days(initial_days as i64)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            format!("{GRAPH}/me/mailFolders/{folder}/messages/delta?$select={SELECT}&$filter=receivedDateTime+ge+{since}")
        }
    };
    for _ in 0..MAX_PAGES {
        let v = match get_json(http, &url, token, &headers).await {
            Ok(v) => v,
            Err(ConnectorError::CursorExpired) if cursor.is_some() => {
                out.reset.push(stream.into());
                out.notes.push(format!("{folder} delta token expired; will re-sync"));
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        for m in v["value"].as_array().cloned().unwrap_or_default() {
            if let Some(msg) = parse_message(&m) {
                out.messages.push(msg);
            }
        }
        if let Some(next) = v["@odata.nextLink"].as_str() {
            url = next.to_string();
            continue;
        }
        if let Some(delta) = v["@odata.deltaLink"].as_str() {
            out.cursors.push((stream.into(), delta.to_string()));
        }
        return Ok(());
    }
    out.notes.push(format!("{folder}: more pages remain; continuing next sync"));
    // Not saving the nextLink as cursor would refetch; saving it continues where we stopped.
    out.cursors.push((stream.into(), url));
    Ok(())
}

fn graph_time(v: &Value) -> Option<DateTime<Utc>> {
    let s = v["dateTime"].as_str()?;
    // We request UTC via Prefer: outlook.timezone="UTC"; values look like 2026-10-19T05:00:00.0000000
    let s = s.split('.').next().unwrap_or(s);
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok().map(|n| n.and_utc())
}

pub fn parse_event(e: &Value) -> Option<IncomingEvent> {
    let id = e["id"].as_str()?;
    let start = graph_time(&e["start"])?;
    let end = graph_time(&e["end"]).unwrap_or(start + Duration::minutes(30));
    let attendees = e["attendees"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|x| x["type"].as_str() != Some("resource"))
                .filter_map(|x| {
                    Some(Attendee {
                        email: x["emailAddress"]["address"].as_str()?.to_lowercase(),
                        name: x["emailAddress"]["name"].as_str().map(|s| s.to_string()),
                        response: x["status"]["response"].as_str().map(|s| s.to_string()),
                        optional: x["type"].as_str() == Some("optional"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let my = e["responseStatus"]["response"].as_str().map(|r| match r {
        "accepted" | "organizer" => "accepted",
        "declined" => "declined",
        "tentativelyAccepted" => "tentative",
        _ => "needsAction",
    });
    let body = e["body"]["content"].as_str().map(|b| if e["body"]["contentType"].as_str() == Some("html") { html_to_text(b) } else { b.to_string() });
    Some(IncomingEvent {
        external_id: id.into(),
        title: e["subject"].as_str().unwrap_or("(no title)").to_string(),
        description: body.or_else(|| e["bodyPreview"].as_str().map(|s| s.to_string())),
        location: e["location"]["displayName"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string()),
        start_at: start,
        end_at: end,
        timezone: e["originalStartTimeZone"].as_str().map(|s| s.to_string()),
        all_day: e["isAllDay"].as_bool().unwrap_or(false),
        organizer_email: e["organizer"]["emailAddress"]["address"].as_str().map(|s| s.to_lowercase()),
        organizer_name: e["organizer"]["emailAddress"]["name"].as_str().map(|s| s.to_string()),
        attendees,
        join_url: e["onlineMeeting"]["joinUrl"].as_str().map(|s| s.to_string()),
        url: e["webLink"].as_str().map(|s| s.to_string()),
        status: if e["isCancelled"].as_bool().unwrap_or(false) { "cancelled".into() } else if my == Some("tentative") { "tentative".into() } else { "confirmed".into() },
        my_response: my.map(|s| s.to_string()),
    })
}

pub async fn sync_calendar(http: &reqwest::Client, token: &str, out: &mut SyncOutput) -> Result<(), ConnectorError> {
    let from = (Utc::now() - Duration::days(2)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let to = (Utc::now() + Duration::days(30)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let headers = [("Prefer", "outlook.timezone=\"UTC\", outlook.body-content-type=\"text\"")];
    let mut url = format!(
        "{GRAPH}/me/calendarView?startDateTime={}&endDateTime={}&$top=100&$select=id,subject,body,bodyPreview,start,end,attendees,organizer,onlineMeeting,webLink,isCancelled,responseStatus,isAllDay,location,originalStartTimeZone",
        urlencoding::encode(&from),
        urlencoding::encode(&to)
    );
    for _ in 0..MAX_PAGES {
        let v = get_json(http, &url, token, &headers).await?;
        for e in v["value"].as_array().cloned().unwrap_or_default() {
            if let Some(ev) = parse_event(&e) {
                out.events.push(ev);
            }
        }
        match v["@odata.nextLink"].as_str() {
            Some(n) => url = n.to_string(),
            None => break,
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
    fn parses_graph_message_and_skips_drafts() {
        let v = json!({"id": "AAM", "conversationId": "C1", "subject": "Budget", "isDraft": false,
            "from": {"emailAddress": {"name": "Omar", "address": "Omar@Corp.com"}},
            "toRecipients": [{"emailAddress": {"name": "Me", "address": "me@corp.com"}}],
            "sentDateTime": "2026-10-13T05:00:00Z", "body": {"contentType": "text", "content": "We still need your approval to move forward."},
            "webLink": "https://outlook.office365.com/owa/?ItemID=AAM"});
        let m = parse_message(&v).unwrap();
        assert_eq!(m.sender_email.as_deref(), Some("omar@corp.com"));
        assert_eq!(m.to, vec!["Me <me@corp.com>".to_string()]);
        let d = json!({"id": "x", "isDraft": true});
        assert!(parse_message(&d).is_none());
        assert!(parse_message(&json!({"id": "y", "@removed": {"reason": "deleted"}})).is_none());
    }

    #[test]
    fn parses_graph_event() {
        let e = json!({"id": "E1", "subject": "Weekly sync", "start": {"dateTime": "2026-10-19T05:00:00.0000000", "timeZone": "UTC"},
            "end": {"dateTime": "2026-10-19T05:30:00.0000000", "timeZone": "UTC"}, "isCancelled": true,
            "responseStatus": {"response": "tentativelyAccepted"}, "attendees": []});
        let ev = parse_event(&e).unwrap();
        assert_eq!(ev.status, "cancelled");
        assert_eq!(ev.my_response.as_deref(), Some("tentative"));
        assert_eq!(ev.start_at.to_rfc3339(), "2026-10-19T05:00:00+00:00");
    }
}
