//! "Connection settings": the app registrations (Google / Microsoft / Slack) that Kept signs in
//! through. Each user — or their IT admin — owns these, so Kept never depends on the publisher.
//! An admin can export them to a file once and hand it to colleagues, who import it.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "kept-connection-settings-v1";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct GoogleApp {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct MicrosoftApp {
    pub client_id: String,
    #[serde(default = "common")]
    pub tenant: String,
}

fn common() -> String {
    "common".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SlackApp {
    pub client_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ConnectionSettings {
    pub format: String,
    #[serde(default)]
    pub organisation: Option<String>,
    #[serde(default)]
    pub google: Option<GoogleApp>,
    #[serde(default)]
    pub microsoft: Option<MicrosoftApp>,
    #[serde(default)]
    pub slack: Option<SlackApp>,
}

pub fn valid_google_id(s: &str) -> bool {
    let s = s.trim();
    s.ends_with(".apps.googleusercontent.com") && s.len() > 30 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
}

pub fn valid_google_secret(s: &str) -> bool {
    let s = s.trim();
    s.len() >= 10 && s.len() <= 100 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn valid_guid(s: &str) -> bool {
    let s = s.trim();
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5 && [8, 4, 4, 4, 12].iter().zip(&parts).all(|(n, p)| p.len() == *n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

pub fn valid_tenant(s: &str) -> bool {
    let s = s.trim();
    matches!(s, "common" | "organizations" | "consumers") || valid_guid(s) || (s.contains('.') && s.len() < 120 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
}

pub fn valid_slack_id(s: &str) -> bool {
    let s = s.trim();
    let mut parts = s.split('.');
    matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None) if !a.is_empty() && !b.is_empty() && a.chars().all(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit()))
}

/// Plain-language problem with a value, or None if it looks right.
pub fn check(kind: &str, value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    let ok = match kind {
        "google_client_id" => valid_google_id(v),
        "google_client_secret" => valid_google_secret(v),
        "microsoft_client_id" => valid_guid(v),
        "microsoft_tenant" => valid_tenant(v),
        "slack_client_id" => valid_slack_id(v),
        _ => true,
    };
    if ok {
        return None;
    }
    Some(match kind {
        "google_client_id" => "A Google client ID ends with .apps.googleusercontent.com — copy the whole value.".into(),
        "google_client_secret" => "That doesn't look like a Google client secret (it usually starts with GOCSPX-).".into(),
        "microsoft_client_id" => "The Application (client) ID looks like 12345678-1234-1234-1234-123456789abc.".into(),
        "microsoft_tenant" => "Use common, organizations, your tenant ID, or your domain (e.g. contoso.com).".into(),
        "slack_client_id" => "A Slack Client ID looks like 1234567890.1234567890 (Basic Information → App Credentials).".into(),
        _ => "Invalid value".into(),
    })
}

pub fn parse(json: &str) -> Result<ConnectionSettings> {
    let s: ConnectionSettings = serde_json::from_str(json).map_err(|_| anyhow::anyhow!("This isn't a Kept connection settings file."))?;
    if s.format != FORMAT {
        bail!("This isn't a Kept connection settings file (or it's from a newer version of Kept).");
    }
    if let Some(g) = &s.google {
        if let Some(e) = check("google_client_id", &g.client_id) {
            bail!("Google: {e}");
        }
        if let Some(e) = g.client_secret.as_deref().and_then(|x| check("google_client_secret", x)) {
            bail!("Google: {e}");
        }
    }
    if let Some(m) = &s.microsoft {
        if let Some(e) = check("microsoft_client_id", &m.client_id).or_else(|| check("microsoft_tenant", &m.tenant)) {
            bail!("Microsoft: {e}");
        }
    }
    if let Some(sl) = &s.slack {
        if let Some(e) = check("slack_client_id", &sl.client_id) {
            bail!("Slack: {e}");
        }
    }
    if s.google.is_none() && s.microsoft.is_none() && s.slack.is_none() {
        bail!("The file doesn't contain any connection settings.");
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_ids() {
        assert!(valid_google_id("123456789012-abcdefghijklmnop.apps.googleusercontent.com"));
        assert!(!valid_google_id("123456"));
        assert!(valid_google_secret("GOCSPX-abcdefghijklmnopqrstuv"));
        assert!(valid_guid("12345678-1234-1234-1234-123456789abc"));
        assert!(!valid_guid("12345678-1234"));
        assert!(valid_tenant("common") && valid_tenant("contoso.com") && valid_tenant("12345678-1234-1234-1234-123456789abc"));
        assert!(valid_slack_id("1234567890.1234567890"));
        assert!(!valid_slack_id("abc"));
        assert!(check("google_client_id", "nope").is_some());
        assert!(check("google_client_id", "").is_none());
    }

    #[test]
    fn round_trip_and_rejects_bad_files() {
        let s = ConnectionSettings {
            format: FORMAT.into(),
            organisation: Some("Acme".into()),
            google: Some(GoogleApp { client_id: "123456789012-abcdefghijklmnop.apps.googleusercontent.com".into(), client_secret: Some("GOCSPX-abcdefghijklmnop".into()) }),
            microsoft: Some(MicrosoftApp { client_id: "12345678-1234-1234-1234-123456789abc".into(), tenant: "contoso.com".into() }),
            slack: None,
        };
        let back = parse(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        assert!(parse("{}").is_err());
        assert!(parse(r#"{"format":"kept-connection-settings-v1"}"#).is_err());
        assert!(parse(r#"{"format":"kept-connection-settings-v1","google":{"client_id":"bad"}}"#).is_err());
    }
}
