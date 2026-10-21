//! Microsoft Teams chats via Microsoft Graph (delegated `Chat.Read`, no admin consent needed).
//! Reads 1:1 and group chats you are part of. Channel messages need the admin-consented
//! `ChannelMessage.Read.All` permission and are not read.

use super::http::{base, get_json, html_to_text};
use super::oauth::ProviderEndpoints;
use super::{ConnectorError, SyncOutput};
use crate::models::IncomingMessage;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use std::collections::HashMap;

pub const SCOPES: &[&str] = &["offline_access", "User.Read", "Chat.Read"];
const MAX_CHATS: usize = 30;

fn graph() -> String {
    base("KEPT_TEST_GRAPH_BASE", "https://graph.microsoft.com/v1.0")
}

pub fn endpoints(client_id: &str, tenant: &str) -> ProviderEndpoints {
    super::microsoft::endpoints_with(client_id, tenant, SCOPES)
}

pub async fn my_id(http: &reqwest::Client, token: &str) -> Result<String, ConnectorError> {
    let v = get_json(http, &format!("{}/me?$select=id", graph()), token, &[]).await?;
    Ok(v["id"].as_str().unwrap_or_default().to_string())
}

fn ts(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&Utc))
}

pub fn parse_message(chat_id: &str, chat_type: &str, m: &Value, members: &HashMap<String, (Option<String>, Option<String>)>, my_email: &str) -> Option<IncomingMessage> {
    if m["messageType"].as_str() != Some("message") || m["deletedDateTime"].is_string() {
        return None;
    }
    let id = m["id"].as_str()?;
    let uid = m["from"]["user"]["id"].as_str()?;
    let (name, email) = members.get(uid).cloned().unwrap_or((m["from"]["user"]["displayName"].as_str().map(|s| s.to_string()), None));
    let body = m["body"]["content"].as_str().unwrap_or_default();
    let text = if m["body"]["contentType"].as_str() == Some("html") { html_to_text(body) } else { body.to_string() };
    if text.trim().is_empty() {
        return None;
    }
    let mut to = vec![my_email.to_string()];
    if chat_type != "oneOnOne" {
        to.extend(members.values().filter_map(|(_, e)| e.clone()).filter(|e| !e.eq_ignore_ascii_case(my_email)));
    }
    Some(IncomingMessage {
        external_id: format!("{chat_id}:{id}"),
        kind: "chat".into(),
        thread_key: Some(format!("teams:{chat_id}:{}", m["replyToId"].as_str().unwrap_or(id))),
        sender_name: name,
        sender_email: email.or_else(|| Some(format!("{uid}@teams"))),
        to,
        cc: vec![],
        subject: m["subject"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string()),
        body_text: text,
        url: m["webUrl"].as_str().map(|s| s.to_string()),
        sent_at: ts(&m["createdDateTime"]).unwrap_or_else(Utc::now),
    })
}

/// Chats with activity since the cursor (RFC 3339), newest messages first.
pub async fn sync(http: &reqwest::Client, token: &str, my_email: &str, cursor: Option<&str>, initial_days: u32, out: &mut SyncOutput) -> Result<(), ConnectorError> {
    let since = cursor.and_then(|c| DateTime::parse_from_rfc3339(c).ok()).map(|d| d.with_timezone(&Utc)).unwrap_or_else(|| Utc::now() - Duration::days(initial_days as i64));
    let chats = get_json(http, &format!("{}/me/chats?$expand=lastMessagePreview&$orderby=lastMessagePreview/createdDateTime%20desc&$top=50", graph()), token, &[]).await?;
    let mut newest = since;
    let mut n = 0;
    for c in chats["value"].as_array().cloned().unwrap_or_default() {
        let Some(cid) = c["id"].as_str() else { continue };
        let last = ts(&c["lastMessagePreview"]["createdDateTime"]);
        if last.map(|l| l <= since).unwrap_or(true) {
            continue;
        }
        n += 1;
        if n > MAX_CHATS {
            out.notes.push("More Teams chats have new messages; they will be read on the next sync".into());
            break;
        }
        let chat_type = c["chatType"].as_str().unwrap_or("group").to_string();
        let mem = get_json(http, &format!("{}/chats/{cid}/members", graph()), token, &[]).await.unwrap_or(Value::Null);
        let mut members = HashMap::new();
        for m in mem["value"].as_array().cloned().unwrap_or_default() {
            if let Some(uid) = m["userId"].as_str() {
                members.insert(uid.to_string(), (m["displayName"].as_str().map(|s| s.to_string()), m["email"].as_str().map(|s| s.to_lowercase())));
            }
        }
        let msgs = match get_json(http, &format!("{}/chats/{cid}/messages?$top=50&$orderby=createdDateTime%20desc", graph()), token, &[]).await {
            Ok(v) => v,
            Err(ConnectorError::CursorExpired) => continue,
            Err(e) => return Err(e),
        };
        for m in msgs["value"].as_array().cloned().unwrap_or_default() {
            let created = ts(&m["createdDateTime"]).unwrap_or(since);
            if created <= since {
                break; // sorted newest first
            }
            newest = newest.max(created);
            if let Some(msg) = parse_message(cid, &chat_type, &m, &members, my_email) {
                out.messages.push(msg);
            }
        }
    }
    out.cursors.push(("chat".into(), newest.to_rfc3339()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_teams_chat_message() {
        let mut members = HashMap::new();
        members.insert("u-sara".to_string(), (Some("Sara".to_string()), Some("sara@corp.com".to_string())));
        members.insert("u-me".to_string(), (Some("Me".to_string()), Some("me@corp.com".to_string())));
        let m = json!({"id": "1700", "messageType": "message", "createdDateTime": "2026-10-18T05:00:00Z",
            "from": {"user": {"id": "u-sara", "displayName": "Sara"}}, "body": {"contentType": "html", "content": "<p>Can you send the <b>budget</b> by Friday?</p>"}});
        let msg = parse_message("19:abc", "oneOnOne", &m, &members, "me@corp.com").unwrap();
        assert_eq!(msg.sender_email.as_deref(), Some("sara@corp.com"));
        assert_eq!(msg.body_text, "Can you send the budget by Friday?");
        assert_eq!(msg.to, vec!["me@corp.com".to_string()]);
        let sys = json!({"id": "1", "messageType": "systemEventMessage", "from": null});
        assert!(parse_message("19:abc", "group", &sys, &members, "me@corp.com").is_none());
    }
}
