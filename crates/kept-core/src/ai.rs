//! Provider-agnostic AI client. The app never depends on one vendor:
//! Anthropic (Claude), OpenAI, or any OpenAI-compatible endpoint (Ollama, LM Studio,
//! OpenRouter, Azure-style gateways) can be configured in Settings.
//!
//! Note: an API key is billed per use by the provider. A consumer chat subscription
//! (e.g. a Claude.ai or ChatGPT plan) does NOT include API access.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("AI provider is not configured")]
    NotConfigured,
    #[error("authentication failed — check the API key")]
    Auth,
    #[error("rate limited by provider (retry after {0:?}s)")]
    RateLimited(Option<u64>),
    #[error("provider timed out")]
    Timeout,
    #[error("provider error {0}: {1}")]
    Http(u16, String),
    #[error("network error: {0}")]
    Network(String),
    #[error("malformed response: {0}")]
    Malformed(String),
}

#[derive(Clone, Debug)]
pub struct AiConfig {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub timeout: Duration,
}

pub fn default_model(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "claude-haiku-5-5",
        "openai" => "gpt-4.1-mini",
        _ => "llama3.1",
    }
}

impl AiConfig {
    pub fn model(&self) -> String {
        if self.model.trim().is_empty() {
            default_model(&self.provider).to_string()
        } else {
            self.model.trim().to_string()
        }
    }
}

/// Send a system + user prompt and return the model's text. JSON is requested.
pub async fn complete(http: &reqwest::Client, cfg: &AiConfig, system: &str, user: &str, max_tokens: u32) -> Result<String, AiError> {
    match cfg.provider.as_str() {
        "anthropic" => anthropic(http, cfg, system, user, max_tokens).await,
        "openai" | "openai_compatible" => openai(http, cfg, system, user, max_tokens).await,
        _ => Err(AiError::NotConfigured),
    }
}

fn map_err(e: reqwest::Error) -> AiError {
    if e.is_timeout() {
        AiError::Timeout
    } else {
        AiError::Network(e.without_url().to_string())
    }
}

async fn check(resp: reqwest::Response) -> Result<Value, AiError> {
    let status = resp.status().as_u16();
    let retry = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let body = resp.text().await.map_err(map_err)?;
    match status {
        200..=299 => serde_json::from_str(&body).map_err(|e| AiError::Malformed(e.to_string())),
        401 | 403 => Err(AiError::Auth),
        429 | 529 => Err(AiError::RateLimited(retry)),
        _ => {
            // Provider error bodies don't contain our secrets, but keep them short.
            let msg = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.pointer("/error/message").and_then(|m| m.as_str()).map(|s| s.to_string()))
                .unwrap_or_else(|| crate::text::truncate(&body, 200));
            Err(AiError::Http(status, msg))
        }
    }
}

async fn anthropic(http: &reqwest::Client, cfg: &AiConfig, system: &str, user: &str, max_tokens: u32) -> Result<String, AiError> {
    let key = cfg.api_key.as_deref().ok_or(AiError::NotConfigured)?;
    let base = if cfg.base_url.trim().is_empty() { "https://api.anthropic.com" } else { cfg.base_url.trim_end_matches('/') };
    let resp = http
        .post(format!("{base}/v1/messages"))
        .timeout(cfg.timeout)
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": cfg.model(),
            "max_tokens": max_tokens,
            "system": system,
            "messages": [{"role": "user", "content": user}]
        }))
        .send()
        .await
        .map_err(map_err)?;
    let v = check(resp).await?;
    let text: String = v["content"]
        .as_array()
        .map(|a| a.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join(""))
        .unwrap_or_default();
    if text.is_empty() {
        return Err(AiError::Malformed("empty content".into()));
    }
    Ok(text)
}

async fn openai(http: &reqwest::Client, cfg: &AiConfig, system: &str, user: &str, max_tokens: u32) -> Result<String, AiError> {
    let base = if cfg.provider == "openai" || cfg.base_url.trim().is_empty() {
        "https://api.openai.com/v1".to_string()
    } else {
        cfg.base_url.trim_end_matches('/').to_string()
    };
    let mut req = http.post(format!("{base}/chat/completions")).timeout(cfg.timeout);
    if let Some(k) = cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    } else if cfg.provider == "openai" {
        return Err(AiError::NotConfigured);
    }
    let resp = req
        .json(&json!({
            "model": cfg.model(),
            "max_completion_tokens": max_tokens,
            "response_format": {"type": "json_object"},
            "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}]
        }))
        .send()
        .await
        .map_err(map_err)?;
    let v = check(resp).await?;
    v.pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AiError::Malformed("missing choices[0].message.content".into()))
}

/// Pull the first top-level JSON object out of model text (tolerates ```json fences / prose).
pub fn extract_json(text: &str) -> Result<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Ok(v);
    }
    let start = t.find('{').ok_or_else(|| anyhow!("no JSON object"))?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, ch) in t[start..].char_indices() {
        if in_str {
            match (esc, ch) {
                (true, _) => esc = false,
                (false, '\\') => esc = true,
                (false, '"') => in_str = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(serde_json::from_str(&t[start..start + i + 1])?);
                }
            }
            _ => {}
        }
    }
    Err(anyhow!("unterminated JSON"))
}

/// Cheap connectivity test used by Settings → "Test connection".
pub async fn test(http: &reqwest::Client, cfg: &AiConfig) -> Result<String, AiError> {
    let out = complete(http, cfg, "Reply with JSON only.", "Return {\"ok\": true}", 20).await?;
    match extract_json(&out) {
        Ok(v) if v["ok"] == json!(true) => Ok(format!("Connected to {} ({})", cfg.provider, cfg.model())),
        _ => Ok(format!("Connected to {} ({}), but the model's reply was not valid JSON — extraction may be unreliable with this model.", cfg.provider, cfg.model())),
    }
}
