//! MOCKED integration tests for Slack, Teams, Google Drive, OneDrive/SharePoint, Jira, Asana,
//! Todoist and Linear: the real sync engines run against a local fake server that returns the
//! response shapes documented by each provider. (Live behaviour is verified by connecting real
//! accounts — see docs/TESTING.md.)

use kept_core::db::{new_id, now, ts};
use kept_core::models::{status, IncomingMessage};
use kept_core::secrets::{self, MemoryStore, SecretStore};
use kept_core::tasks::{self, TaskFilter};
use kept_core::{documents, ingest, Agent};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Handler = Arc<dyn Fn(&str, &str, &str, &str) -> (u16, String) + Send + Sync>;

async fn serve(h: Handler) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { break };
            let h = h.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
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
                        let mut first = head.lines().next().unwrap_or("").split_whitespace();
                        let method = first.next().unwrap_or("").to_string();
                        let path = first.next().unwrap_or("").to_string();
                        let auth = head.lines().find_map(|l| l.strip_prefix("authorization: ").or(l.strip_prefix("Authorization: "))).unwrap_or("").to_string();
                        let (code, out) = h(&method, &path, &body, &auth);
                        let resp = format!("HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len());
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

fn iso(days: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::days(days)).format("%Y-%m-%d").to_string()
}

struct State {
    todoist_round: AtomicUsize,
    created: Mutex<Vec<Value>>,
}

fn handler(st: Arc<State>) -> Handler {
    Arc::new(move |method, path, body, auth| {
        let ok = |v: Value| (200, v.to_string());
        let now_ts = chrono::Utc::now().timestamp();
        // ---------------- Todoist
        if path.starts_with("/todoist/") {
            if !auth.contains("tdtoken123") {
                return (401, "{}".into());
            }
            if path.starts_with("/todoist/user") {
                return ok(json!({"email": "dana@corp.example", "full_name": "Dana Reyes"}));
            }
            if path.starts_with("/todoist/projects") {
                return ok(json!({"results": [{"id": "p1", "name": "Work"}], "next_cursor": null}));
            }
            if path.starts_with("/todoist/tasks") && method == "POST" {
                st.created.lock().unwrap().push(serde_json::from_str(body).unwrap_or(Value::Null));
                return ok(json!({"id": "td-new", "content": "x"}));
            }
            if path.starts_with("/todoist/tasks") {
                let round = st.todoist_round.load(Ordering::SeqCst);
                let mut results = vec![json!({"id": "td2", "content": "Renew passport", "project_id": "p1", "priority": 4, "due": {"date": iso(2)}, "checked": round >= 1})];
                if round < 2 {
                    results.push(json!({"id": "td1", "content": "Book flights to Dubai", "project_id": "p1", "priority": 1, "due": null, "checked": false}));
                }
                return ok(json!({"results": results, "next_cursor": null}));
            }
        }
        // ---------------- Jira
        if path.starts_with("/jira/rest/api/3/myself") {
            return if auth.starts_with("Basic ") { ok(json!({"emailAddress": "dana@corp.example", "displayName": "Dana Reyes"})) } else { (401, "{}".into()) };
        }
        if path.starts_with("/jira/rest/api/3/search/jql") && method == "POST" {
            assert!(body.contains("currentUser()"));
            return ok(json!({"issues": [
                {"id": "1001", "key": "OPS-7", "fields": {"summary": "Rotate production API keys", "duedate": iso(3), "status": {"name": "To Do", "statusCategory": {"key": "new"}}, "project": {"name": "Ops"}, "priority": {"name": "High"}}},
                {"id": "1002", "key": "OPS-8", "fields": {"summary": "Old resolved thing", "status": {"name": "Done", "statusCategory": {"key": "done"}}, "project": {"name": "Ops"}}}
            ]}));
        }
        // ---------------- Asana
        if path.starts_with("/asana/users/me") {
            return ok(json!({"data": {"gid": "u1", "name": "Dana Reyes", "email": "dana@corp.example", "workspaces": [{"gid": "w1", "name": "Acme"}]}}));
        }
        if path.starts_with("/asana/tasks") {
            assert!(path.contains("assignee=me"));
            return ok(json!({"data": [{"gid": "a1", "name": "Write launch FAQ", "completed": false, "due_on": iso(4), "permalink_url": "https://app.asana.com/0/1/a1", "projects": [{"name": "Launch"}]}], "next_page": null}));
        }
        // ---------------- Linear
        if path.starts_with("/linear") {
            if auth != "lin_api_abcdefgh" {
                return ok(json!({"errors": [{"message": "Authentication required"}]}));
            }
            if body.contains("assignedIssues") {
                return ok(json!({"data": {"viewer": {"assignedIssues": {"nodes": [{"id": "l1", "identifier": "ENG-12", "title": "Fix SSO redirect loop", "url": "https://linear.app/acme/issue/ENG-12", "dueDate": iso(1), "priority": 1, "state": {"name": "In Progress", "type": "started"}, "team": {"name": "Eng"}}]}}}}));
            }
            return ok(json!({"data": {"viewer": {"id": "v1", "name": "Dana Reyes", "email": "dana@corp.example"}}}));
        }
        // ---------------- Slack
        if path.starts_with("/slack/") {
            let m = path.trim_start_matches("/slack/").split('?').next().unwrap_or("");
            return ok(match m {
                "conversations.list" => json!({"ok": true, "channels": [{"id": "D1", "is_im": true, "user": "U2", "updated": now_ts}]}),
                "conversations.history" => json!({"ok": true, "messages": [
                    {"type": "message", "user": "U2", "ts": format!("{}.000100", now_ts - 600), "text": "hey <@UME>, can you send the Q4 numbers by Friday?"},
                    {"type": "message", "subtype": "channel_join", "user": "U2", "ts": format!("{}.000200", now_ts - 500), "text": "joined"}
                ]}),
                "users.info" => {
                    if path.contains("U2") {
                        json!({"ok": true, "user": {"name": "sara", "profile": {"real_name": "Sara Lindqvist", "email": "sara@corp.example"}}})
                    } else {
                        json!({"ok": true, "user": {"name": "dana", "profile": {"real_name": "Dana Reyes", "display_name": "dana", "email": "dana@corp.example"}}})
                    }
                }
                "search.messages" => {
                    if path.contains("from%3A") {
                        json!({"ok": true, "messages": {"matches": [{"ts": format!("{}.000300", now_ts - 400), "user": "UME", "text": "I'll update the hiring roadmap tomorrow.", "channel": {"id": "C9", "name": "leads"}, "permalink": "https://acme.slack.com/archives/C9/p1"}]}})
                    } else {
                        json!({"ok": true, "messages": {"matches": [{"ts": format!("{}.000400", now_ts - 300), "user": "U2", "text": "<@UME> please review the launch plan before Thursday", "channel": {"id": "C7", "name": "launch"}, "permalink": "https://acme.slack.com/archives/C7/p2"}]}})
                    }
                }
                _ => json!({"ok": false, "error": "unknown_method"}),
            });
        }
        // ---------------- Microsoft Graph (Teams + OneDrive)
        if path.starts_with("/graph/me/chats") {
            return ok(json!({"value": [{"id": "19:chat1", "chatType": "oneOnOne", "lastMessagePreview": {"createdDateTime": chrono::Utc::now().to_rfc3339()}}]}));
        }
        if path.starts_with("/graph/chats/19:chat1/members") {
            return ok(json!({"value": [{"userId": "u-omar", "displayName": "Omar Haddad", "email": "omar@corp.example"}, {"userId": "u-dana", "displayName": "Dana Reyes", "email": "dana@corp.example"}]}));
        }
        if path.starts_with("/graph/chats/19:chat1/messages") {
            return ok(json!({"value": [{"id": "171", "messageType": "message", "createdDateTime": (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339(),
                "from": {"user": {"id": "u-omar", "displayName": "Omar Haddad"}}, "body": {"contentType": "html", "content": "<p>We still need your approval on the vendor contract.</p>"}}]}));
        }
        if path.starts_with("/graph/search/query") && method == "POST" {
            return ok(json!({"value": [{"hitsContainers": [{"moreResultsAvailable": false, "hits": [{"resource": {"id": "od1", "name": "Q4 numbers summary.xlsx", "webUrl": "https://contoso.sharepoint.com/q4", "lastModifiedDateTime": chrono::Utc::now().to_rfc3339(), "file": {"mimeType": "x"}}}]}]}]}));
        }
        // ---------------- Google Drive
        if path.starts_with("/gdrive/files") {
            return ok(json!({"files": [{"id": "g1", "name": "Launch plan v3.docx", "mimeType": "application/vnd.google-apps.document", "modifiedTime": chrono::Utc::now().to_rfc3339(), "webViewLink": "https://docs.google.com/d/g1"}]}));
        }
        (404, "{}".into())
    })
}

fn insert_oauth_account(a: &Agent, store: &MemoryStore, provider: &str, email: &str, streams: &str, config: Value) -> String {
    let id = new_id();
    a.db(|c| {
        c.execute(
            "INSERT INTO accounts(id, provider, category, display_name, email, status, enabled_streams, connected_at, config_json) VALUES (?1,?2,'chat',?3,?4,'connected',?5,?6,?7)",
            rusqlite::params![id, provider, format!("{provider} test"), email, streams, ts(&now()), config.to_string()],
        )?;
        Ok(())
    })
    .unwrap();
    let token = json!({"access_token": "tok", "refresh_token": "r", "expires_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(), "scope": "x"});
    store.set(&secrets::token_key_name(&id), &token.to_string()).unwrap();
    id
}

fn titles(a: &Agent) -> Vec<kept_core::models::Task> {
    a.db(|c| tasks::list(c, &TaskFilter { include_closed: true, ..Default::default() })).unwrap()
}

#[tokio::test]
async fn every_connector_end_to_end_against_mock_apis() {
    let st = Arc::new(State { todoist_round: AtomicUsize::new(0), created: Mutex::new(vec![]) });
    let base = serve(handler(st.clone())).await;
    for (k, v) in [("KEPT_TEST_TODOIST_BASE", "todoist"), ("KEPT_TEST_ASANA_BASE", "asana"), ("KEPT_TEST_LINEAR_BASE", "linear"), ("KEPT_TEST_SLACK_BASE", "slack"), ("KEPT_TEST_GRAPH_BASE", "graph"), ("KEPT_TEST_GDRIVE_BASE", "gdrive")] {
        std::env::set_var(k, format!("{base}/{v}"));
    }
    let store = Arc::new(MemoryStore::default());
    let a = Agent::in_memory(store.clone()).unwrap();
    let mut p = a.prefs().unwrap();
    p.user_name = "Dana Reyes".into();
    p.user_emails = vec!["dana@corp.example".into()];
    p.timezone = "Asia/Karachi".into();
    a.save_prefs(&p).unwrap();

    // A commitment detected from email first, so the Todoist task must merge with it.
    let acct = new_id();
    a.db(|c| {
        c.execute("INSERT INTO accounts(id, provider, category, display_name, status, connected_at) VALUES (?1,'import','multi','x','connected',?2)", rusqlite::params![acct, ts(&now())])?;
        let p = kept_core::prefs::load(c)?;
        ingest::store_messages(c, &acct, "import", &p, &[IncomingMessage {
            external_id: "e1".into(), kind: "email".into(), thread_key: Some("t1".into()), sender_name: Some("Kim".into()), sender_email: Some("kim@corp.example".into()),
            to: vec!["dana@corp.example".into()], subject: Some("Trip".into()), body_text: "Can you book flights to Dubai for the offsite?".into(), sent_at: chrono::Utc::now() - chrono::Duration::hours(2), ..Default::default()
        }])?;
        Ok(())
    })
    .unwrap();
    a.process_pending(10).await.unwrap();
    assert!(titles(&a).iter().any(|t| t.title.to_lowercase().contains("flights to dubai")));

    // ---------------- Todoist: token check, mirror + dedup, completion evidence, vanish → review, push
    assert!(a.connect_with_token("todoist", "wrongtoken", None, None).await.is_err(), "bad token rejected");
    let td = a.connect_with_token("todoist", "tdtoken123", None, None).await.unwrap();
    let r = a.sync_account(&td.id).await.unwrap();
    assert!(r.contains("1 new tasks") && r.contains("1 linked"), "{r}");
    let all = titles(&a);
    assert_eq!(all.iter().filter(|t| t.title.to_lowercase().contains("flights to dubai")).count(), 1, "no duplicate of the detected task");
    let passport = all.iter().find(|t| t.title == "Renew passport").unwrap();
    assert_eq!(passport.origin, "external");
    assert_eq!(passport.priority, 1, "Todoist p1 → critical");
    assert_eq!(passport.due_kind, "explicit");
    st.todoist_round.store(1, Ordering::SeqCst);
    a.sync_account(&td.id).await.unwrap();
    let passport = a.db(|c| tasks::get(c, &passport.id)).unwrap();
    assert_eq!(passport.status, status::COMPLETED);
    assert!(passport.completion_evidence.unwrap().contains("Todoist"));
    st.todoist_round.store(2, Ordering::SeqCst);
    a.sync_account(&td.id).await.unwrap();
    let flights = titles(&a).into_iter().find(|t| t.title.to_lowercase().contains("flights to dubai")).unwrap();
    assert!(flights.needs_confirmation, "vanished from Todoist → asked, not auto-completed");
    assert_ne!(flights.status, status::COMPLETED);
    let manual = a.db(|c| tasks::create(c, &tasks::NewTask { title: "Prepare offsite agenda".into(), due_at: Some(chrono::Utc::now() + chrono::Duration::days(3)), ..Default::default() }, "user")).unwrap();
    a.push_to_todoist(&manual, None).await.unwrap();
    assert_eq!(st.created.lock().unwrap()[0]["content"], "Prepare offsite agenda");
    assert!(a.push_to_todoist(&manual, None).await.is_err(), "no double push");
    let executed: i64 = a.db(|c| Ok(c.query_row("SELECT COUNT(*) FROM external_actions WHERE state='executed'", [], |r| r.get(0))?)).unwrap();
    assert_eq!(executed, 1);

    // ---------------- Jira (Basic auth, JQL search, done issues not imported)
    let jira = a.connect_with_token("jira", "jiratoken123", Some("dana@corp.example"), Some(&format!("{base}/jira"))).await.unwrap();
    a.sync_account(&jira.id).await.unwrap();
    let all = titles(&a);
    let rot = all.iter().find(|t| t.title == "Rotate production API keys").expect("jira issue mirrored");
    assert!(a.db(|c| tasks::detail(c, &rot.id)).unwrap().sources.iter().any(|s| s.url.as_deref().unwrap_or("").ends_with("/browse/OPS-7")));
    assert!(!all.iter().any(|t| t.title == "Old resolved thing"));

    // ---------------- Asana
    let asana = a.connect_with_token("asana", "asanatoken123", None, None).await.unwrap();
    a.sync_account(&asana.id).await.unwrap();
    assert!(titles(&a).iter().any(|t| t.title == "Write launch FAQ" && t.project.as_deref() == Some("Launch")));

    // ---------------- Linear (raw key header)
    assert!(a.connect_with_token("linear", "lin_api_wrong", None, None).await.is_err());
    let lin = a.connect_with_token("linear", "lin_api_abcdefgh", None, None).await.unwrap();
    a.sync_account(&lin.id).await.unwrap();
    assert!(titles(&a).iter().any(|t| t.title == "Fix SSO redirect loop"));

    // ---------------- Slack: DM request, channel mention, my own promise in a channel
    let sl = insert_oauth_account(&a, &store, "slack", "dana@corp.example", "[\"chat\"]", json!({"user_id": "UME", "team": "Acme", "team_url": "https://acme.slack.com/", "name": "Dana Reyes"}));
    let r = a.sync_account(&sl.clone()).await.unwrap();
    assert!(r.contains("3 new messages"), "{r}");
    a.process_pending(50).await.unwrap();
    let all = titles(&a);
    assert!(all.iter().any(|t| t.title.contains("Q4 numbers")), "{:#?}", all.iter().map(|t| &t.title).collect::<Vec<_>>());
    assert!(all.iter().any(|t| t.title.to_lowercase().contains("launch plan")));
    assert!(all.iter().any(|t| t.title.to_lowercase().contains("hiring roadmap")));
    let cursor: String = a.db(|c| Ok(c.query_row("SELECT cursor FROM sync_cursors WHERE account_id=?1", [&sl], |r| r.get(0))?)).unwrap();
    assert!(cursor.parse::<f64>().unwrap() > 0.0);

    // ---------------- Teams chat
    let tm = insert_oauth_account(&a, &store, "teams", "dana@corp.example", "[\"chat\"]", json!({}));
    a.sync_account(&tm).await.unwrap();
    a.process_pending(50).await.unwrap();
    assert!(titles(&a).iter().any(|t| t.task_type == "approval" && t.title.to_lowercase().contains("approval")));

    // ---------------- Documents: OneDrive/SharePoint + Google Drive, related lookup
    let od = insert_oauth_account(&a, &store, "onedrive", "dana@corp.example", "[\"documents\"]", json!({}));
    assert!(a.sync_account(&od).await.unwrap().contains("1 documents"));
    let gd = insert_oauth_account(&a, &store, "gdrive", "dana@corp.example", "[\"documents\"]", json!({}));
    a.sync_account(&gd).await.unwrap();
    let rel = a.db(|c| documents::related(c, "Send the Q4 numbers to Sara", 5)).unwrap();
    assert_eq!(rel.first().map(|d| d.name.as_str()), Some("Q4 numbers summary.xlsx"));
    let rel = a.db(|c| documents::related(c, "Review the launch plan", 5)).unwrap();
    assert_eq!(rel.first().map(|d| d.name.as_str()), Some("Launch plan v3.docx"));
    assert!(a.db(|c| documents::related(c, "Book flights", 5)).unwrap().is_empty());

    // ---------------- Disconnect removes mirrored tasks and indexed documents
    a.disconnect(&jira.id, true).await.unwrap();
    assert!(!titles(&a).iter().any(|t| t.title == "Rotate production API keys"));
    a.disconnect(&od, true).await.unwrap();
    let docs: i64 = a.db(|c| Ok(c.query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))?)).unwrap();
    assert_eq!(docs, 1);
}
