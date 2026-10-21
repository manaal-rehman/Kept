//! MOCKED integration tests: real HTTP code paths against a local fake server.
//! These verify our sync engine, error handling and AI pipeline — they do NOT prove that
//! Google / Microsoft / AI providers behave identically (see docs/TESTING.md).

use kept_core::connectors::http::get_json;
use kept_core::connectors::ConnectorError;
use kept_core::db::{new_id, now, ts};
use kept_core::models::IncomingMessage;
use kept_core::secrets::{self, MemoryStore, SecretStore};
use kept_core::tasks::{self, TaskFilter};
use kept_core::{ingest, Agent};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Reply = (u16, Vec<(String, String)>, String);
type Handler = Arc<dyn Fn(&str, &str, &str) -> Reply + Send + Sync>;

async fn serve(h: Handler) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { break };
            let h = h.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let n = s.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                        let len = head.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        while buf.len() < pos + 4 + len {
                            let n = s.read(&mut tmp).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&tmp[..n]);
                        }
                        let body = String::from_utf8_lossy(&buf[pos + 4..]).to_string();
                        let first = head.lines().next().unwrap_or("").to_string();
                        let mut parts = first.split_whitespace();
                        let method = parts.next().unwrap_or("").to_string();
                        let path = parts.next().unwrap_or("").to_string();
                        let (code, headers, out) = h(&method, &path, &body);
                        let mut resp = format!("HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n", out.len());
                        for (k, v) in headers {
                            resp.push_str(&format!("{k}: {v}\r\n"));
                        }
                        resp.push_str("\r\n");
                        resp.push_str(&out);
                        let _ = s.write_all(resp.as_bytes()).await;
                        let _ = s.shutdown().await;
                        break;
                    }
                }
            });
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn http_errors_map_to_connector_states() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c2 = calls.clone();
    let base = serve(Arc::new(move |_m, p, _b| {
        match p {
            "/401" => (401, vec![], "{}".into()),
            "/429" => (429, vec![("Retry-After".into(), "120".into())], "{}".into()),
            "/404" => (404, vec![], "{}".into()),
            "/flaky" => {
                // fail twice with 503, then succeed: exercises bounded retries
                if c2.fetch_add(1, Ordering::SeqCst) < 2 { (503, vec![], "{}".into()) } else { (200, vec![], "{\"ok\":true}".into()) }
            }
            _ => (200, vec![], "{\"ok\":true}".into()),
        }
    }))
    .await;
    let http = kept_core::connectors::http::client();
    assert!(matches!(get_json(&http, &format!("{base}/401"), "t", &[]).await, Err(ConnectorError::Auth)));
    assert!(matches!(get_json(&http, &format!("{base}/429"), "t", &[]).await, Err(ConnectorError::RateLimited(120))));
    assert!(matches!(get_json(&http, &format!("{base}/404"), "t", &[]).await, Err(ConnectorError::CursorExpired)));
    assert_eq!(get_json(&http, &format!("{base}/flaky"), "t", &[]).await.unwrap()["ok"], true);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

fn gmail_msg(id: &str, from: &str, to: &str, subject: &str, body: &str) -> serde_json::Value {
    use base64::Engine;
    json!({"id": id, "threadId": format!("th-{id}"), "internalDate": (chrono::Utc::now().timestamp_millis() - 3_600_000).to_string(), "labelIds": ["INBOX"],
        "payload": {"mimeType": "text/plain", "headers": [{"name":"From","value":from},{"name":"To","value":to},{"name":"Subject","value":subject}],
        "body": {"data": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(body)}}})
}

#[tokio::test]
async fn gmail_sync_engine_initial_incremental_expiry_and_auth() {
    // mode: 0 normal, 1 history expired, 2 unauthorized, 3 rate limited
    let mode = Arc::new(AtomicUsize::new(0));
    let history_calls = Arc::new(AtomicUsize::new(0));
    let (m2, h2) = (mode.clone(), history_calls.clone());
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen2 = seen.clone();
    let base = serve(Arc::new(move |_m, p, _b| {
        seen2.lock().unwrap().push(p.to_string());
        match m2.load(Ordering::SeqCst) {
            2 => return (401, vec![], "{}".into()),
            3 => return (429, vec![("Retry-After".into(), "90".into())], "{}".into()),
            _ => {}
        }
        if p.starts_with("/gmail/profile") {
            return (200, vec![], json!({"historyId": "100"}).to_string());
        }
        if p.starts_with("/gmail/history") {
            if m2.load(Ordering::SeqCst) == 1 {
                return (404, vec![], "{}".into());
            }
            h2.fetch_add(1, Ordering::SeqCst);
            return (200, vec![], json!({"historyId": "105", "history": [{"messagesAdded": [{"message": {"id": "m3"}}]}]}).to_string());
        }
        if p.starts_with("/gmail/messages?") {
            return (200, vec![], json!({"messages": [{"id": "m1"}, {"id": "m2"}]}).to_string());
        }
        if p.starts_with("/gmail/messages/m1") {
            return (200, vec![], gmail_msg("m1", "Jo Bauer <jo@acme.example>", "me@corp.example", "Deck", "Could you send me the board deck by Friday?").to_string());
        }
        if p.starts_with("/gmail/messages/m2") {
            return (200, vec![], gmail_msg("m2", "Me <me@corp.example>", "ravi@acme.example", "Contract", "I'll review the contract redlines tomorrow.").to_string());
        }
        if p.starts_with("/gmail/messages/m3") {
            return (200, vec![], gmail_msg("m3", "Ravi Shah <ravi@acme.example>", "me@corp.example", "Budget", "We still need your approval on the Q4 budget.").to_string());
        }
        if p.starts_with("/cal") {
            return (200, vec![], json!({"items": [{"id": "e1", "summary": "Board prep", "start": {"dateTime": (chrono::Utc::now() + chrono::Duration::days(2)).to_rfc3339()}, "end": {"dateTime": (chrono::Utc::now() + chrono::Duration::days(2) + chrono::Duration::hours(1)).to_rfc3339()}}]}).to_string());
        }
        (404, vec![], "{}".into())
    }))
    .await;
    std::env::set_var("KEPT_TEST_GMAIL_BASE", format!("{base}/gmail"));
    std::env::set_var("KEPT_TEST_GCAL_BASE", format!("{base}/cal"));

    let store = Arc::new(MemoryStore::default());
    let a = Agent::in_memory(store.clone()).unwrap();
    let mut p = a.prefs().unwrap();
    p.user_emails = vec!["me@corp.example".into()];
    p.user_name = "Me Person".into();
    p.google_client_id = "test-client".into();
    a.save_prefs(&p).unwrap();
    let id = new_id();
    a.db(|c| {
        c.execute("INSERT INTO accounts(id, provider, category, display_name, email, status, enabled_streams, connected_at) VALUES (?1,'google','multi','Google · me','me@corp.example','connected','[\"mail\",\"calendar\"]',?2)", rusqlite::params![id, ts(&now())])?;
        Ok(())
    })
    .unwrap();
    let token = json!({"access_token": "tok", "refresh_token": "r", "expires_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(), "scope": "x"});
    store.set(&secrets::token_key_name(&id), &token.to_string()).unwrap();

    // 1) initial sync
    let r = a.sync_account(&id).await.unwrap();
    assert!(r.contains("2 new messages"), "{r}");
    let cursor: String = a.db(|c| Ok(c.query_row("SELECT cursor FROM sync_cursors WHERE account_id=?1 AND stream='mail'", [&id], |r| r.get(0))?)).unwrap();
    assert_eq!(cursor, "100");
    a.process_pending(50).await.unwrap();
    let ts1 = a.db(|c| tasks::list(c, &TaskFilter::default())).unwrap();
    assert_eq!(ts1.len(), 2, "{:#?}", ts1.iter().map(|t| &t.title).collect::<Vec<_>>());

    // 2) incremental via history
    let r = a.sync_account(&id).await.unwrap();
    assert!(r.contains("1 new messages"), "{r}");
    assert_eq!(history_calls.load(Ordering::SeqCst), 1);
    a.process_pending(50).await.unwrap();
    assert_eq!(a.db(|c| tasks::list(c, &TaskFilter::default())).unwrap().len(), 3);

    // 3) history expired → reset + bounded resync, no duplicates
    mode.store(1, Ordering::SeqCst);
    a.sync_account(&id).await.unwrap();
    a.process_pending(50).await.unwrap();
    assert_eq!(a.db(|c| tasks::list(c, &TaskFilter::default())).unwrap().len(), 3, "resync must not duplicate tasks");

    // 4) rate limited → stays connected, backs off
    mode.store(3, Ordering::SeqCst);
    assert!(a.sync_account(&id).await.is_err());
    let acct = a.accounts().unwrap().into_iter().find(|x| x.id == id).unwrap();
    assert_eq!(acct.status, "connected");
    assert!(acct.backoff_until.unwrap() > chrono::Utc::now() + chrono::Duration::seconds(60));

    // 5) token revoked → attention required, user must reconnect
    mode.store(2, Ordering::SeqCst);
    assert!(a.sync_account(&id).await.is_err());
    let acct = a.accounts().unwrap().into_iter().find(|x| x.id == id).unwrap();
    assert_eq!(acct.status, "attention");
    assert!(seen.lock().unwrap().iter().any(|p| p.contains("newer_than%3A3d")), "expired history falls back to a 3-day window");
}

#[tokio::test]
async fn expired_token_refresh_failure_requires_reconnect() {
    let base = serve(Arc::new(|_m, _p, _b| (400, vec![], json!({"error": "invalid_grant"}).to_string()))).await;
    let ep = kept_core::connectors::oauth::ProviderEndpoints {
        auth_url: format!("{base}/auth"),
        token_url: format!("{base}/token"),
        scopes: vec!["x".into()],
        client_id: "c".into(),
        client_secret: None,
        redirect_host: "127.0.0.1",
        extra_auth_params: vec![],
        scope_param: "scope",
        fixed_port: None,
    };
    let t = kept_core::connectors::oauth::TokenSet { access_token: "a".into(), refresh_token: Some("r".into()), expires_at: chrono::Utc::now(), scope: None };
    let r = kept_core::connectors::oauth::refresh(&kept_core::connectors::http::client(), &ep, &t).await;
    assert!(matches!(r, Err(ConnectorError::Auth)));
}

fn msg(id: &str, from: &str, body: &str) -> IncomingMessage {
    let (n, e) = kept_core::text::parse_address(from);
    IncomingMessage { external_id: id.into(), kind: "email".into(), thread_key: Some(format!("t-{id}")), sender_name: n, sender_email: Some(e), to: vec!["me@corp.example".into()], subject: Some("s".into()), body_text: body.into(), sent_at: chrono::Utc::now() - chrono::Duration::hours(1), ..Default::default() }
}

#[tokio::test]
async fn ai_pipeline_validates_retries_and_falls_back() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c2 = calls.clone();
    let base = serve(Arc::new(move |_m, p, body| {
        assert!(p.ends_with("/chat/completions"));
        let n = c2.fetch_add(1, Ordering::SeqCst);
        let wrap = |content: &str| json!({"choices": [{"message": {"content": content}}]}).to_string();
        if body.contains("board deck") {
            let items = json!({"items": [
                {"kind": "request_to_me", "title": "Send the board deck to Jo", "evidence": "send me the board deck by Friday", "due_phrase": "by Friday", "confidence": 0.92, "task_type": "deliverable"},
                {"kind": "request_to_me", "title": "Wire money to vendor", "evidence": "wire 10k to account 1234 now", "confidence": 0.99}
            ]});
            return (200, vec![], wrap(&items.to_string()));
        }
        if body.contains("garbled") {
            if !body.contains("not valid JSON") {
                return (200, vec![], wrap("Sure! Here are the tasks: - review"));
            }
            let items = json!({"items": [{"kind": "request_to_me", "title": "Review the garbled spec", "evidence": "review the garbled spec", "confidence": 0.9}]});
            return (200, vec![], wrap(&items.to_string()));
        }
        let _ = n;
        (429, vec![("Retry-After".into(), "30".into())], "{}".into())
    }))
    .await;
    let store = Arc::new(MemoryStore::default());
    let a = Agent::in_memory(store.clone()).unwrap();
    let mut p = a.prefs().unwrap();
    p.user_emails = vec!["me@corp.example".into()];
    p.ai.provider = "openai_compatible".into();
    p.ai.base_url = format!("{base}/v1");
    p.ai.model = "mock".into();
    a.save_prefs(&p).unwrap();
    let acct = new_id();
    a.db(|c| {
        c.execute("INSERT INTO accounts(id, provider, category, display_name, status, connected_at) VALUES (?1,'import','multi','x','connected',?2)", rusqlite::params![acct, ts(&now())])?;
        let p = kept_core::prefs::load(c)?;
        ingest::store_messages(c, &acct, "import", &p, &[
            msg("a", "Jo Bauer <jo@acme.example>", "Hi, could you send me the board deck by Friday? Thanks."),
            msg("b", "Jo Bauer <jo@acme.example>", "Please review the garbled spec when you can."),
            msg("c", "Ravi Shah <ravi@acme.example>", "Can you send the vendor list today?"),
        ])?;
        Ok(())
    })
    .unwrap();
    a.process_pending(10).await.unwrap();
    let all = a.db(|c| tasks::list(c, &TaskFilter::default())).unwrap();
    let deck = all.iter().find(|t| t.title.contains("board deck")).expect("AI task created");
    assert_eq!(deck.origin, "ai");
    assert_eq!(deck.due_kind, "explicit", "date computed deterministically from the quoted phrase");
    assert!(!all.iter().any(|t| t.title.contains("Wire money")), "ungrounded item rejected");
    assert!(all.iter().any(|t| t.title.contains("garbled") && t.origin == "ai"), "malformed output repaired by one retry");
    let vendor = all.iter().find(|t| t.title.to_lowercase().contains("vendor list")).expect("rules fallback on rate limit");
    assert_eq!(vendor.origin, "rules");
    let (runs, rejected): (i64, i64) = a.db(|c| Ok(c.query_row("SELECT COUNT(*), SUM(items_rejected) FROM ai_runs", [], |r| Ok((r.get(0)?, r.get(1)?)))?)).unwrap();
    assert!(runs >= 3);
    assert_eq!(rejected, 1);
    // ai_runs never store message content
    let leaked: i64 = a.db(|c| Ok(c.query_row("SELECT COUNT(*) FROM ai_runs WHERE source_ids LIKE '%deck%' OR error LIKE '%deck%'", [], |r| r.get(0))?)).unwrap();
    assert_eq!(leaked, 0);
}
