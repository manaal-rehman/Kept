//! Shared HTTP handling for provider APIs: status mapping, Retry-After, bounded retries.

use super::ConnectorError;
use serde_json::Value;
use std::time::Duration;

/// API base URL; debug builds allow an override so the engines can be tested against a mock server.
pub fn base(var: &str, default: &str) -> String {
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var(var) {
        return v;
    }
    let _ = var;
    default.to_string()
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Kept/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(40))
        .connect_timeout(Duration::from_secs(15))
        .pool_idle_timeout(Duration::from_secs(30))
        .build()
        .expect("http client")
}

fn map_status(status: u16, retry_after: Option<u64>, body: &str) -> ConnectorError {
    match status {
        401 => ConnectorError::Auth,
        403 if body.contains("rateLimitExceeded") || body.contains("userRateLimitExceeded") => ConnectorError::RateLimited(retry_after.unwrap_or(60)),
        403 => ConnectorError::Permanent(format!("permission denied by provider: {}", short(body))),
        404 | 410 => ConnectorError::CursorExpired,
        429 => ConnectorError::RateLimited(retry_after.unwrap_or(60)),
        500..=599 => ConnectorError::Transient(format!("provider returned {status}")),
        _ => ConnectorError::Permanent(format!("provider returned {status}: {}", short(body))),
    }
}

fn short(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(|m| m.as_str()).map(|s| s.to_string()).or_else(|| v.get("error_description").and_then(|m| m.as_str()).map(|s| s.to_string())))
        .unwrap_or_else(|| crate::text::truncate(body, 160))
}

/// GET JSON with a bearer token. Retries transient failures twice with backoff.
pub async fn get_json(http: &reqwest::Client, url: &str, token: &str, headers: &[(&str, &str)]) -> Result<Value, ConnectorError> {
    let mut attempt = 0;
    loop {
        let mut req = http.get(url).bearer_auth(token);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let res = req.send().await;
        match res {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let retry = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
                let body = resp.text().await.map_err(|e| ConnectorError::Transient(e.without_url().to_string()))?;
                if (200..300).contains(&status) {
                    return serde_json::from_str(&body).map_err(|e| ConnectorError::Transient(format!("invalid JSON: {e}")));
                }
                let err = map_status(status, retry, &body);
                match &err {
                    ConnectorError::Transient(_) if attempt < 2 => {}
                    ConnectorError::RateLimited(s) if attempt < 1 && *s <= 10 => tokio::time::sleep(Duration::from_secs(*s)).await,
                    _ => return Err(err),
                }
            }
            Err(e) => {
                if attempt >= 2 {
                    return Err(ConnectorError::Transient(e.without_url().to_string()));
                }
            }
        }
        attempt += 1;
        tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await;
    }
}

pub async fn post_form(http: &reqwest::Client, url: &str, form: &[(&str, &str)]) -> Result<Value, ConnectorError> {
    let resp = http.post(url).form(form).send().await.map_err(|e| ConnectorError::Transient(e.without_url().to_string()))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| ConnectorError::Transient(e.without_url().to_string()))?;
    if (200..300).contains(&status) {
        return serde_json::from_str(&body).map_err(|e| ConnectorError::Transient(format!("invalid JSON: {e}")));
    }
    if status == 400 && (body.contains("invalid_grant") || body.contains("interaction_required")) {
        return Err(ConnectorError::Auth);
    }
    Err(map_status(status, None, &body))
}

/// Very small HTML → text for email bodies that have no text/plain part.
pub fn html_to_text(html: &str) -> String {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static SCRIPT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<(script|style|head)[^>]*>.*?</(script|style|head)>").unwrap());
    static BR: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)<(br|/p|/div|/li|/tr|/h\d)[^>]*>").unwrap());
    static BLOCKQUOTE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<blockquote[^>]*>.*?</blockquote>").unwrap());
    static TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<[^>]+>").unwrap());
    let s = SCRIPT.replace_all(html, "");
    let s = BLOCKQUOTE.replace_all(&s, "\n");
    let s = BR.replace_all(&s, "\n");
    let s = TAG.replace_all(&s, "");
    let s = s.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&rsquo;", "'");
    let lines: Vec<&str> = s.lines().map(|l| l.trim()).collect();
    let mut out = String::new();
    let mut blank = 0;
    for l in lines {
        if l.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(l);
        out.push('\n');
    }
    out.trim().to_string()
}

pub enum Auth<'a> {
    Bearer(&'a str),
    Basic(&'a str, &'a str),
    /// Raw Authorization header value (Linear personal API keys).
    Raw(&'a str),
}

/// Send a request (GET/POST, optional JSON body) with the same status mapping and retries as `get_json`.
pub async fn request_json(http: &reqwest::Client, method: reqwest::Method, url: &str, auth: Auth<'_>, body: Option<&Value>, headers: &[(&str, &str)]) -> Result<Value, ConnectorError> {
    let mut attempt = 0;
    loop {
        let mut req = http.request(method.clone(), url);
        req = match auth {
            Auth::Bearer(t) => req.bearer_auth(t),
            Auth::Basic(u, p) => req.basic_auth(u, Some(p)),
            Auth::Raw(v) => req.header("Authorization", v),
        };
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        if let Some(b) = body {
            req = req.json(b);
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let retry = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
                let text = resp.text().await.map_err(|e| ConnectorError::Transient(e.without_url().to_string()))?;
                if (200..300).contains(&status) {
                    if text.trim().is_empty() {
                        return Ok(Value::Null);
                    }
                    return serde_json::from_str(&text).map_err(|e| ConnectorError::Transient(format!("invalid JSON: {e}")));
                }
                let err = map_status(status, retry, &text);
                match &err {
                    ConnectorError::Transient(_) if attempt < 2 => {}
                    _ => return Err(err),
                }
            }
            Err(e) => {
                if attempt >= 2 {
                    return Err(ConnectorError::Transient(e.without_url().to_string()));
                }
            }
        }
        attempt += 1;
        tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await;
    }
}
