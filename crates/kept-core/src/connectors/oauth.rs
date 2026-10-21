//! OAuth 2.0 Authorization Code flow with PKCE (RFC 7636) and a loopback redirect
//! (RFC 8252 "OAuth for Native Apps"). The user signs in in their own browser;
//! Kept never sees passwords. Tokens are stored only in the OS credential store.

use super::http::post_form;
use super::ConnectorError;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub scope: Option<String>,
}

impl TokenSet {
    pub fn fresh(&self) -> bool {
        self.expires_at - Duration::seconds(90) > Utc::now()
    }
}

#[derive(Debug, Clone)]
pub struct ProviderEndpoints {
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    pub client_id: String,
    pub client_secret: Option<String>,
    /// "127.0.0.1" (Google) or "localhost" (Microsoft) — must match the app registration.
    pub redirect_host: &'static str,
    pub extra_auth_params: Vec<(String, String)>,
    /// Query parameter that carries scopes ("scope", or Slack's "user_scope").
    pub scope_param: &'static str,
    /// Some providers (Slack) need the exact registered redirect, including the port.
    pub fixed_port: Option<u16>,
}

impl ProviderEndpoints {
    pub fn standard(auth_url: String, token_url: String, scopes: Vec<String>, client_id: String, client_secret: Option<String>, redirect_host: &'static str) -> Self {
        Self { auth_url, token_url, scopes, client_id, client_secret, redirect_host, extra_auth_params: vec![], scope_param: "scope", fixed_port: None }
    }
}

pub struct Pending {
    pub auth_url: String,
    pub redirect_uri: String,
    pub verifier: String,
    pub state: String,
    pub listener: TcpListener,
}

fn random_b64(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Start a sign-in: bind a loopback port and build the browser URL.
pub async fn begin(p: &ProviderEndpoints) -> Result<Pending, ConnectorError> {
    let addr = format!("127.0.0.1:{}", p.fixed_port.unwrap_or(0));
    let listener = TcpListener::bind(&addr).await.map_err(|e| {
        ConnectorError::Permanent(match p.fixed_port {
            Some(port) => format!("port {port} is in use by another program; close it and try again ({e})"),
            None => format!("cannot open loopback port: {e}"),
        })
    })?;
    let port = listener.local_addr().map_err(|e| ConnectorError::Permanent(e.to_string()))?.port();
    let redirect_uri = format!("http://{}:{port}/callback", p.redirect_host);
    let verifier = random_b64(48);
    let state = random_b64(24);
    let mut url = url::Url::parse(&p.auth_url).map_err(|e| ConnectorError::Permanent(e.to_string()))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", &p.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_type", "code")
            .append_pair(p.scope_param, &p.scopes.join(if p.scope_param == "user_scope" { "," } else { " " }))
            .append_pair("code_challenge", &challenge(&verifier))
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);
        for (k, v) in &p.extra_auth_params {
            q.append_pair(k, v);
        }
    }
    Ok(Pending { auth_url: url.to_string(), redirect_uri, verifier, state, listener })
}

const DONE_HTML: &str = "<!doctype html><html><head><meta charset=utf-8><title>Kept</title><style>body{font-family:Segoe UI,system-ui,sans-serif;background:#f6f5f2;color:#1d1c1a;display:grid;place-items:center;height:100vh;margin:0}div{text-align:center}h1{font-weight:600;font-size:20px}</style></head><body><div><h1>You're connected.</h1><p>You can close this tab and return to Kept.</p></div></body></html>";
const FAIL_HTML: &str = "<!doctype html><html><head><meta charset=utf-8><title>Kept</title></head><body style=\"font-family:Segoe UI,system-ui,sans-serif;padding:40px\"><h1>Sign-in was not completed</h1><p>Return to Kept and try again.</p></body></html>";

/// Wait for the browser redirect and return the authorization code.
pub async fn wait_for_code(pending: &Pending, timeout: std::time::Duration) -> Result<String, ConnectorError> {
    let fut = async {
        loop {
            let (mut sock, _) = pending.listener.accept().await.map_err(|e| ConnectorError::Transient(e.to_string()))?;
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("").to_string();
            if !path.starts_with("/callback") {
                let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                continue;
            }
            let url = url::Url::parse(&format!("http://localhost{path}")).map_err(|e| ConnectorError::Permanent(e.to_string()))?;
            let mut code = None;
            let mut state = None;
            let mut error = None;
            for (k, v) in url.query_pairs() {
                match k.as_ref() {
                    "code" => code = Some(v.to_string()),
                    "state" => state = Some(v.to_string()),
                    "error" => error = Some(v.to_string()),
                    _ => {}
                }
            }
            let ok = error.is_none() && code.is_some() && state.as_deref() == Some(pending.state.as_str());
            let html = if ok { DONE_HTML } else { FAIL_HTML };
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{html}", html.len());
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.shutdown().await;
            if let Some(e) = error {
                return Err(ConnectorError::Permanent(format!("sign-in was cancelled or denied ({e})")));
            }
            if state.as_deref() != Some(pending.state.as_str()) {
                return Err(ConnectorError::Permanent("state mismatch — possible CSRF; sign-in aborted".into()));
            }
            return code.ok_or_else(|| ConnectorError::Permanent("no authorization code returned".into()));
        }
    };
    tokio::time::timeout(timeout, fut).await.map_err(|_| ConnectorError::Permanent("sign-in timed out".into()))?
}

fn parse_tokens(v: serde_json::Value, prev_refresh: Option<String>) -> Result<TokenSet, ConnectorError> {
    // Slack answers HTTP 200 with {"ok": false, "error": ...}.
    if v.get("ok") == Some(&serde_json::Value::Bool(false)) {
        let err = v["error"].as_str().unwrap_or("unknown_error");
        return Err(if err.contains("invalid") || err.contains("revoked") || err.contains("expired") { ConnectorError::Auth } else { ConnectorError::Permanent(format!("sign-in failed: {err}")) });
    }
    // Slack puts user tokens under "authed_user" on the first exchange.
    let v = if v.get("access_token").and_then(|a| a.as_str()).is_none() && v.get("authed_user").is_some() { v["authed_user"].clone() } else { v };
    let access = v["access_token"].as_str().ok_or_else(|| ConnectorError::Permanent("token response missing access_token".into()))?;
    let exp = v["expires_in"].as_i64().unwrap_or(3600);
    Ok(TokenSet {
        access_token: access.to_string(),
        refresh_token: v["refresh_token"].as_str().map(|s| s.to_string()).or(prev_refresh),
        expires_at: Utc::now() + Duration::seconds(exp),
        scope: v["scope"].as_str().map(|s| s.to_string()),
    })
}

pub async fn exchange(http: &reqwest::Client, p: &ProviderEndpoints, pending: &Pending, code: &str) -> Result<TokenSet, ConnectorError> {
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", &pending.redirect_uri),
        ("client_id", &p.client_id),
        ("code_verifier", &pending.verifier),
    ];
    let scopes = p.scopes.join(" ");
    if p.redirect_host == "localhost" {
        form.push(("scope", &scopes));
    }
    if let Some(s) = &p.client_secret {
        form.push(("client_secret", s));
    }
    let v = post_form(http, &p.token_url, &form).await?;
    parse_tokens(v, None)
}

pub async fn refresh(http: &reqwest::Client, p: &ProviderEndpoints, t: &TokenSet) -> Result<TokenSet, ConnectorError> {
    let rt = t.refresh_token.as_deref().ok_or(ConnectorError::Auth)?;
    let scopes = p.scopes.join(" ");
    let mut form: Vec<(&str, &str)> = vec![("grant_type", "refresh_token"), ("refresh_token", rt), ("client_id", &p.client_id)];
    if p.redirect_host == "localhost" {
        form.push(("scope", &scopes));
    }
    if let Some(s) = &p.client_secret {
        form.push(("client_secret", s));
    }
    let v = post_form(http, &p.token_url, &form).await?;
    parse_tokens(v, t.refresh_token.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        // RFC 7636 Appendix B
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[tokio::test]
    async fn loopback_flow_validates_state() {
        let p = ProviderEndpoints {
            auth_url: "https://example.com/auth".into(),
            token_url: "https://example.com/token".into(),
            scopes: vec!["a".into(), "b".into()],
            client_id: "cid".into(),
            client_secret: None,
            redirect_host: "127.0.0.1",
            extra_auth_params: vec![],
            scope_param: "scope",
            fixed_port: None,
        };
        let pending = begin(&p).await.unwrap();
        assert!(pending.auth_url.contains("code_challenge_method=S256"));
        let mut slack = p.clone();
        slack.scope_param = "user_scope";
        slack.fixed_port = Some(0);
        assert!(begin(&slack).await.unwrap().auth_url.contains("user_scope=a%2Cb"));
        assert!(pending.auth_url.contains("scope=a+b"));
        let port = pending.listener.local_addr().unwrap().port();
        let state = pending.state.clone();
        let client = tokio::spawn(async move {
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(format!("GET /callback?code=abc123&state={state} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await.unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).await.unwrap();
            out
        });
        let code = wait_for_code(&pending, std::time::Duration::from_secs(5)).await.unwrap();
        assert_eq!(code, "abc123");
        assert!(client.await.unwrap().contains("connected"));

        let pending = begin(&p).await.unwrap();
        let port = pending.listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(b"GET /callback?code=abc&state=WRONG HTTP/1.1\r\n\r\n").await.unwrap();
        });
        assert!(wait_for_code(&pending, std::time::Duration::from_secs(5)).await.is_err());
    }
}
