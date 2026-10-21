//! Slack connector (user token via OAuth v2 + PKCE, no client secret).
//!
//! Reads, with your own permissions:
//!   * direct messages and group DMs (conversations.history),
//!   * channel messages that @mention you, and your own channel posts (search.messages),
//! so promises you make in channels and requests made to you are both found.
//! Read-only scopes; Kept never posts.

use super::http::{base, get_json};
use super::oauth::ProviderEndpoints;
use super::{ConnectorError, SyncOutput};
use crate::models::IncomingMessage;
use chrono::{DateTime, Duration, TimeZone, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;

pub const USER_SCOPES: &[&str] = &[
    "im:history", "im:read", "mpim:history", "mpim:read", "channels:history", "groups:history", "users:read", "users:read.email", "search:read",
];
/// Slack matches the redirect exactly, so Kept uses a fixed loopback port. Register
/// `http://localhost:53682/callback` as the redirect URL in your Slack app.
pub const REDIRECT_PORT: u16 = 53682;
const MAX_CONVERSATIONS: usize = 40;
const MAX_USER_LOOKUPS: usize = 60;

fn api() -> String {
    base("KEPT_TEST_SLACK_BASE", "https://slack.com/api")
}

pub fn endpoints(client_id: &str) -> ProviderEndpoints {
    ProviderEndpoints {
        auth_url: "https://slack.com/oauth/v2/authorize".into(),
        token_url: format!("{}/oauth.v2.access", api()),
        scopes: USER_SCOPES.iter().map(|s| s.to_string()).collect(),
        client_id: client_id.into(),
        client_secret: None,
        redirect_host: "localhost",
        extra_auth_params: vec![],
        scope_param: "user_scope",
        fixed_port: Some(REDIRECT_PORT),
    }
}

/// Slack returns HTTP 200 with {"ok": false, "error": "..."}; turn that into a ConnectorError.
async fn call(http: &reqwest::Client, token: &str, method: &str, query: &str) -> Result<Value, ConnectorError> {
    let v = get_json(http, &format!("{}/{method}?{query}", api()), token, &[]).await?;
    if v["ok"].as_bool() == Some(true) {
        return Ok(v);
    }
    let err = v["error"].as_str().unwrap_or("unknown_error");
    Err(match err {
        "invalid_auth" | "not_authed" | "token_revoked" | "token_expired" | "account_inactive" => ConnectorError::Auth,
        "ratelimited" => ConnectorError::RateLimited(60),
        "missing_scope" => ConnectorError::Permanent("Slack app is missing a required permission — re-check the scopes in docs/SETUP.md and reconnect.".into()),
        other => ConnectorError::Permanent(format!("Slack error: {other}")),
    })
}

pub struct Me {
    pub user_id: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub handles: Vec<String>,
    pub team: String,
    pub team_url: String,
}

pub async fn whoami(http: &reqwest::Client, token: &str) -> Result<Me, ConnectorError> {
    let a = call(http, token, "auth.test", "").await?;
    let uid = a["user_id"].as_str().unwrap_or_default().to_string();
    let u = call(http, token, "users.info", &format!("user={uid}")).await.ok();
    let profile = u.as_ref().map(|u| u["user"]["profile"].clone()).unwrap_or(Value::Null);
    let mut handles = vec![];
    for k in ["display_name", "real_name"] {
        if let Some(h) = profile[k].as_str().filter(|h| !h.is_empty()) {
            handles.push(h.to_string());
        }
    }
    if let Some(n) = u.as_ref().and_then(|u| u["user"]["name"].as_str()) {
        handles.push(n.to_string());
    }
    Ok(Me {
        user_id: uid,
        email: profile["email"].as_str().map(|s| s.to_lowercase()),
        name: profile["real_name"].as_str().map(|s| s.to_string()),
        handles,
        team: a["team"].as_str().unwrap_or("Slack").to_string(),
        team_url: a["url"].as_str().unwrap_or("https://slack.com/").to_string(),
    })
}

static RE_USER: Lazy<Regex> = Lazy::new(|| Regex::new(r"<@([UW][A-Z0-9]+)(?:\|([^>]+))?>").unwrap());
static RE_CHANNEL: Lazy<Regex> = Lazy::new(|| Regex::new(r"<#[CG][A-Z0-9]+\|([^>]*)>").unwrap());
static RE_LINK: Lazy<Regex> = Lazy::new(|| Regex::new(r"<(https?://[^|>]+)(?:\|([^>]+))?>").unwrap());
static RE_SPECIAL: Lazy<Regex> = Lazy::new(|| Regex::new(r"<!(here|channel|everyone)[^>]*>").unwrap());

/// Turn Slack markup into readable text: <@U123> → @Sara, <#C1|general> → #general, links → text.
pub fn render_text(text: &str, users: &HashMap<String, (Option<String>, Option<String>)>) -> String {
    let t = RE_USER.replace_all(text, |c: &regex::Captures| {
        let id = &c[1];
        let name = users.get(id).and_then(|u| u.0.clone()).or_else(|| c.get(2).map(|m| m.as_str().to_string())).unwrap_or_else(|| id.to_string());
        format!("@{name}")
    });
    let t = RE_CHANNEL.replace_all(&t, "#$1");
    let t = RE_LINK.replace_all(&t, |c: &regex::Captures| c.get(2).map(|m| m.as_str().to_string()).unwrap_or_else(|| c[1].to_string()));
    let t = RE_SPECIAL.replace_all(&t, "@$1");
    t.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
}

pub fn ts_to_time(ts: &str) -> DateTime<Utc> {
    let secs: f64 = ts.parse().unwrap_or(0.0);
    Utc.timestamp_opt(secs as i64, ((secs.fract()) * 1e9) as u32).single().unwrap_or_else(Utc::now)
}

struct Users<'a> {
    http: &'a reqwest::Client,
    token: &'a str,
    map: HashMap<String, (Option<String>, Option<String>)>, // id -> (name, email)
    lookups: usize,
}

impl Users<'_> {
    async fn get(&mut self, id: &str) -> (Option<String>, Option<String>) {
        if let Some(u) = self.map.get(id) {
            return u.clone();
        }
        if self.lookups >= MAX_USER_LOOKUPS {
            return (None, None);
        }
        self.lookups += 1;
        let u = match call(self.http, self.token, "users.info", &format!("user={id}")).await {
            Ok(v) => {
                let p = &v["user"]["profile"];
                let name = p["real_name"].as_str().filter(|s| !s.is_empty()).or(p["display_name"].as_str()).map(|s| s.to_string());
                (name, p["email"].as_str().map(|s| s.to_lowercase()))
            }
            Err(_) => (None, None),
        };
        self.map.insert(id.to_string(), u.clone());
        u
    }
    async fn resolve_mentions(&mut self, text: &str) {
        let ids: Vec<String> = RE_USER.captures_iter(text).map(|c| c[1].to_string()).collect();
        for id in ids {
            self.get(&id).await;
        }
    }
}

fn permalink(team_url: &str, channel: &str, ts: &str) -> String {
    format!("{}archives/{channel}/p{}", if team_url.ends_with('/') { team_url.to_string() } else { format!("{team_url}/") }, ts.replace('.', ""))
}

/// Sync DMs, group DMs, mentions and own channel posts newer than the cursor (a Slack ts).
pub async fn sync(http: &reqwest::Client, token: &str, me: &Me, my_email: &str, cursor: Option<&str>, initial_days: u32, out: &mut SyncOutput) -> Result<(), ConnectorError> {
    let oldest: f64 = cursor.and_then(|c| c.parse().ok()).unwrap_or_else(|| (Utc::now() - Duration::days(initial_days as i64)).timestamp() as f64);
    let mut users = Users { http, token, map: HashMap::new(), lookups: 0 };
    users.map.insert(me.user_id.clone(), (me.name.clone(), Some(my_email.to_string())));
    let mut newest = oldest;
    let mut seen = std::collections::HashSet::new();

    // 1. Direct and group messages.
    let convs = call(http, token, "conversations.list", "types=im,mpim&exclude_archived=true&limit=200").await?;
    let mut list: Vec<Value> = convs["channels"].as_array().cloned().unwrap_or_default();
    list.sort_by(|a, b| b["updated"].as_f64().unwrap_or(0.0).partial_cmp(&a["updated"].as_f64().unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
    for c in list.iter().take(MAX_CONVERSATIONS) {
        let Some(cid) = c["id"].as_str() else { continue };
        let is_im = c["is_im"].as_bool().unwrap_or(false);
        let hist = match call(http, token, "conversations.history", &format!("channel={cid}&oldest={oldest:.6}&limit=100")).await {
            Ok(h) => h,
            Err(ConnectorError::Permanent(_)) => continue,
            Err(e) => return Err(e),
        };
        for m in hist["messages"].as_array().cloned().unwrap_or_default() {
            if m["subtype"].is_string() && m["subtype"] != "thread_broadcast" {
                continue; // joins, bot messages, edits notices
            }
            let (Some(ts), Some(uid)) = (m["ts"].as_str(), m["user"].as_str()) else { continue };
            if !seen.insert(format!("{cid}:{ts}")) {
                continue;
            }
            let text = m["text"].as_str().unwrap_or_default();
            users.resolve_mentions(text).await;
            let (name, email) = users.get(uid).await;
            let mut to = vec![my_email.to_string()];
            if !is_im {
                to.push("group-members@slack".into()); // several recipients → ownership may be shared
            }
            let t = ts_to_time(ts);
            newest = newest.max(ts.parse().unwrap_or(0.0));
            out.messages.push(IncomingMessage {
                external_id: format!("{cid}:{ts}"),
                kind: "chat".into(),
                thread_key: Some(format!("slack:{cid}:{}", m["thread_ts"].as_str().unwrap_or(ts))),
                sender_name: name,
                sender_email: email.or_else(|| Some(format!("{uid}@slack"))),
                to,
                cc: vec![],
                subject: None,
                body_text: render_text(text, &users.map),
                url: Some(permalink(&me.team_url, cid, ts)),
                sent_at: t,
            });
        }
    }

    // 2. Channel messages mentioning me, and my own channel posts (promises).
    let after = (Utc.timestamp_opt(oldest as i64, 0).single().unwrap_or_else(Utc::now) - Duration::days(1)).format("%Y-%m-%d");
    for (q, mine) in [(format!("<@{}> after:{after}", me.user_id), false), (format!("from:<@{}> after:{after}", me.user_id), true)] {
        let r = call(http, token, "search.messages", &format!("query={}&count=100&sort=timestamp&sort_dir=desc", urlencoding::encode(&q))).await?;
        for m in r["messages"]["matches"].as_array().cloned().unwrap_or_default() {
            let (Some(ts), Some(cid)) = (m["ts"].as_str(), m["channel"]["id"].as_str()) else { continue };
            if ts.parse::<f64>().unwrap_or(0.0) <= oldest || m["channel"]["is_im"].as_bool() == Some(true) || m["channel"]["is_mpim"].as_bool() == Some(true) {
                continue;
            }
            if !seen.insert(format!("{cid}:{ts}")) {
                continue;
            }
            let uid = m["user"].as_str().unwrap_or_default();
            let text = m["text"].as_str().unwrap_or_default();
            users.resolve_mentions(text).await;
            let (name, email) = if mine { (me.name.clone(), Some(my_email.to_string())) } else { users.get(uid).await };
            newest = newest.max(ts.parse().unwrap_or(0.0));
            let thread = m["permalink"].as_str().and_then(|p| url::Url::parse(p).ok()).and_then(|u| u.query_pairs().find(|(k, _)| k == "thread_ts").map(|(_, v)| v.to_string()));
            out.messages.push(IncomingMessage {
                external_id: format!("{cid}:{ts}"),
                kind: "chat".into(),
                thread_key: Some(format!("slack:{cid}:{}", thread.as_deref().unwrap_or(ts))),
                sender_name: name,
                sender_email: email.or_else(|| Some(format!("{uid}@slack"))),
                // Channel post: nobody is a direct recipient; ownership comes from @mentions.
                to: vec![],
                cc: vec![],
                subject: m["channel"]["name"].as_str().map(|n| format!("#{n}")),
                body_text: render_text(text, &users.map),
                url: m["permalink"].as_str().map(|s| s.to_string()).or_else(|| Some(permalink(&me.team_url, cid, ts))),
                sent_at: ts_to_time(ts),
            });
        }
    }
    out.cursors.push(("chat".into(), format!("{newest:.6}")));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_slack_markup() {
        let mut u = HashMap::new();
        u.insert("U1".to_string(), (Some("Sara Lindqvist".to_string()), None));
        let t = render_text("<@U1> can you check <#C9|launch> and <https://x.io/a|the doc> &amp; reply? <!here>", &u);
        assert_eq!(t, "@Sara Lindqvist can you check #launch and the doc & reply? @here");
    }

    #[test]
    fn ts_and_permalink() {
        assert_eq!(ts_to_time("1760000000.000200").timestamp(), 1760000000);
        assert_eq!(permalink("https://acme.slack.com/", "C1", "1760000000.000200"), "https://acme.slack.com/archives/C1/p1760000000000200");
    }
}
