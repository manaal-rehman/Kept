//! End-to-end behaviour on the synthetic workspace: extraction → tasks → lifecycle →
//! meetings → notifications → persistence. No network, no real accounts.

use chrono::{Duration, Utc};
use kept_core::models::status;
use kept_core::secrets::MemoryStore;
use kept_core::tasks::{self, TaskFilter, TaskPatch};
use kept_core::{meetings, notify, planner, Agent};
use std::sync::Arc;

async fn sample_agent() -> Arc<Agent> {
    let a = Agent::in_memory(Arc::new(MemoryStore::default())).unwrap();
    let mut p = a.prefs().unwrap();
    p.timezone = "Asia/Karachi".into();
    p.quiet_hours_enabled = false;
    a.save_prefs(&p).unwrap();
    a.load_sample().unwrap();
    a.process_pending(200).await.unwrap();
    a
}

fn all(a: &Agent) -> Vec<kept_core::models::Task> {
    a.db(|c| tasks::list(c, &TaskFilter { include_closed: true, ..Default::default() })).unwrap()
}

fn find(a: &Agent, needle: &str) -> kept_core::models::Task {
    all(a).into_iter().find(|t| t.title.to_lowercase().contains(needle)).unwrap_or_else(|| panic!("no task containing {needle}: {:#?}", all(a).iter().map(|t| &t.title).collect::<Vec<_>>()))
}

fn none(a: &Agent, needle: &str) {
    assert!(!all(a).iter().any(|t| t.title.to_lowercase().contains(needle)), "unexpected task containing '{needle}'");
}

#[tokio::test]
async fn explicit_and_implicit_commitments() {
    let a = sample_agent().await;
    let t = find(&a, "revised report");
    assert_eq!(t.due_kind, "inferred", "\"tomorrow\" in my own promise is an inferred date");
    assert_eq!(t.status, status::READY);
    let t = find(&a, "finance team");
    assert!(t.due_at.is_none(), "no deadline must be invented");
    let d = a.db(|c| tasks::detail(c, &t.id)).unwrap();
    assert!(d.deadlines.iter().any(|x| x.kind == "suggested"), "a suggestion is offered, separately");
    let t = find(&a, "q4 budget summary");
    assert!(t.needs_confirmation, "implicit ownership goes to review");
    let t = find(&a, "approval decision");
    assert_eq!(t.task_type, "approval");
}

#[tokio::test]
async fn ownership_and_negatives() {
    let a = sample_agent().await;
    none(&a, "signed contract"); // only CC'd, addressed to Omar
    none(&a, "contractor"); // hypothetical
    none(&a, "maintenance"); // unrelated deadline
    none(&a, "invoice"); // completed, nothing needed
    none(&a, "roadmap"); // past event
    none(&a, "meeting room"); // someone else's promise
    none(&a, "catering");
    // Other people's promises are tracked as waiting-for, not my tasks.
    let theirs: i64 = a.db(|c| Ok(c.query_row("SELECT COUNT(*) FROM commitments WHERE direction='theirs'", [], |r| r.get(0))?)).unwrap();
    assert_eq!(theirs, 2);
}

#[tokio::test]
async fn meeting_relative_deadline_is_before_the_meeting() {
    let a = sample_agent().await;
    let t = find(&a, "prepare the slides");
    let m = a.db(|c| meetings::get(c, t.meeting_id.as_deref().unwrap())).unwrap();
    assert_eq!(m.title, "Project Review");
    assert!(t.due_at.unwrap() < m.start_at, "prep must be due before the meeting starts");
    assert_eq!((m.start_at - t.due_at.unwrap()).num_minutes(), 30);
}

#[tokio::test]
async fn duplicates_across_sources_merge_and_deadline_updates_are_traceable() {
    let a = sample_agent().await;
    let n = all(&a).iter().filter(|t| t.title.to_lowercase().contains("updated analysis")).count();
    assert_eq!(n, 1, "email + revised email + chat reminder must be one task");
    let t = find(&a, "updated analysis");
    let d = a.db(|c| tasks::detail(c, &t.id)).unwrap();
    assert!(d.sources.len() >= 3);
    assert_eq!(d.deadlines.iter().filter(|x| x.superseded_at.is_none() && x.kind != "suggested").count(), 1);
    assert!(d.deadlines.iter().any(|x| x.superseded_at.is_some()), "old deadline kept in history");
    assert!(t.due_phrase.as_deref().unwrap().contains("wednesday"));
}

#[tokio::test]
async fn followup_finds_original_commitment() {
    let a = sample_agent().await;
    let t = find(&a, "market sizing");
    let d = a.db(|c| tasks::detail(c, &t.id)).unwrap();
    assert!(d.sources.iter().any(|s| s.relation == "followup"));
    assert_eq!(all(&a).iter().filter(|t| t.title.to_lowercase().contains("market sizing")).count(), 1);
    assert_eq!(t.priority, 1, "overdue + chased = top priority");
}

#[tokio::test]
async fn draft_sent_is_not_completion_but_sent_receipts_are() {
    let a = sample_agent().await;
    let t = find(&a, "partnership proposal");
    assert_eq!(t.status, status::AWAITING_APPROVAL);
    assert!(t.completion_evidence.unwrap().contains("not complete"));
    let t = find(&a, "travel receipts");
    assert_eq!(t.status, status::COMPLETED);
    assert!(t.completion_evidence.unwrap().contains("sent"));
}

#[tokio::test]
async fn agreement_reply_creates_task_without_deadline() {
    let a = sample_agent().await;
    let t = find(&a, "onboarding checklist");
    assert!(t.due_at.is_none());
    assert_eq!(t.my_role, "owner");
}

#[tokio::test]
async fn prompt_injection_is_contained() {
    let a = sample_agent().await;
    let t = find(&a, "delivery address");
    assert!(t.needs_confirmation);
    assert!(t.review_reason.unwrap().to_lowercase().contains("injection"));
    // The injected instruction did not change anything else.
    none(&a, "forward all emails");
    let closed = all(&a).iter().filter(|t| t.status == status::COMPLETED).count();
    assert_eq!(closed, 1);
}

#[tokio::test]
async fn meeting_without_agenda_is_flagged_without_invented_prep() {
    let a = sample_agent().await;
    let ms = a.db(|c| {
        meetings::refresh_prep_states(c)?;
        meetings::list_range(c, Utc::now(), Utc::now() + Duration::days(10))
    }).unwrap();
    let m = ms.iter().find(|m| m.title == "Sync with Marketing").unwrap();
    assert_eq!(m.prep_state, "unknown");
    let p = a.prefs().unwrap();
    let b = a.db(|c| meetings::briefing(c, &p, &m.id)).unwrap();
    assert!(!b.has_agenda);
    assert!(b.linked_tasks.is_empty());
    // Only real, sourced items: open promises to these attendees — nothing invented.
    assert!(b.checklist.iter().filter(|i| i.basis == "task").all(|i| i.text.starts_with("Status of:")));
    assert!(b.checklist.iter().any(|i| i.basis == "suggestion"), "suggestions are labelled as such");
    let r = ms.iter().find(|m| m.title == "Project Review").unwrap();
    assert_eq!(r.prep_state, "needed");
}

#[tokio::test]
async fn completion_requires_evidence_and_undo_works() {
    let a = sample_agent().await;
    let t = find(&a, "finance team");
    let err = a.db(|c| tasks::set_status(c, &t.id, status::COMPLETED, "agent", "", None));
    assert!(err.is_err(), "the agent cannot complete a task without evidence");
    a.db(|c| tasks::set_status(c, &t.id, status::COMPLETED, "user", "", None)).unwrap();
    let d = a.db(|c| tasks::detail(c, &t.id)).unwrap();
    assert!(d.task.completion_evidence.unwrap().contains("by you"));
    let ev = d.history.iter().find(|h| h.action == "status").unwrap();
    a.db(|c| tasks::undo(c, &ev.id)).unwrap();
    assert_eq!(a.db(|c| tasks::get(c, &t.id)).unwrap().status, status::READY);
}

#[tokio::test]
async fn user_confirmed_deadline_is_not_silently_overwritten() {
    let a = sample_agent().await;
    let t = find(&a, "revised report");
    let mine = Utc::now() + Duration::days(5);
    a.db(|c| tasks::update(c, &t.id, &TaskPatch { due_at: Some(Some(mine)), ..Default::default() }, "user")).unwrap();
    let r = a.db(|c| tasks::apply_deadline(c, &t.id, "explicit", Utc::now() + Duration::days(1), Some("tomorrow"), None, Utc::now(), true)).unwrap();
    assert!(matches!(r, tasks::DeadlineOutcome::Conflict(_)));
    let t2 = a.db(|c| tasks::get(c, &t.id)).unwrap();
    assert_eq!(t2.due_at.unwrap().timestamp(), mine.timestamp());
    assert!(t2.conflict_note.is_some() && t2.needs_confirmation);
}

#[tokio::test]
async fn merge_and_reject() {
    let a = sample_agent().await;
    let k = find(&a, "revised report");
    let d = find(&a, "pricing doc");
    a.db(|c| tasks::merge(c, &k.id, &d.id, "user")).unwrap();
    let dd = a.db(|c| tasks::get(c, &d.id)).unwrap();
    assert_eq!(dd.status, status::CANCELLED);
    assert!(a.db(|c| tasks::detail(c, &k.id)).unwrap().sources.len() >= 2);
    let r = find(&a, "delivery address");
    a.db(|c| tasks::reject(c, &r.id, "spam")).unwrap();
    assert!(!a.db(|c| tasks::list(c, &TaskFilter::default())).unwrap().iter().any(|t| t.id == r.id));
}

#[tokio::test]
async fn reprocessing_the_same_sync_does_not_duplicate() {
    let a = sample_agent().await;
    let before = all(&a).len();
    a.load_sample().unwrap(); // same messages arrive again in another sync cycle
    let (n, _) = a.process_pending(200).await.unwrap();
    assert_eq!(n, 0, "identical messages are not re-processed");
    assert_eq!(all(&a).len(), before);
}

#[tokio::test]
async fn notifications_dedupe_snooze_and_quiet_hours() {
    let a = sample_agent().await;
    let mut p = a.prefs().unwrap();
    let now = Utc::now();
    let c1 = a.db(|c| notify::generate(c, &p, now)).unwrap();
    let c2 = a.db(|c| notify::generate(c, &p, now + Duration::minutes(1))).unwrap();
    assert!(c1 > 0);
    assert_eq!(c2, 0, "same state, same minute window → no new notifications");
    let toasts = a.db(|c| notify::due_toasts(c, &p, now)).unwrap();
    assert!(!toasts.is_empty());
    assert!(toasts.len() as u32 <= p.max_notifications_per_hour, "rate limit respected");
    let again = a.db(|c| notify::due_toasts(c, &p, now + Duration::minutes(1))).unwrap();
    assert!(again.iter().all(|t| !toasts.iter().any(|x| x.notification_id == t.notification_id)), "a toast is never shown twice");

    // Snooze does not change the task.
    let n = a.db(|c| notify::list(c, "all", 100)).unwrap().into_iter().find(|n| n.entity_type.as_deref() == Some("task")).unwrap();
    let task_before = a.db(|c| tasks::get(c, n.entity_id.as_deref().unwrap())).unwrap();
    a.db(|c| notify::snooze(c, &n.id, 60)).unwrap();
    a.db(|c| notify::dismiss(c, &n.id)).unwrap();
    let task_after = a.db(|c| tasks::get(c, n.entity_id.as_deref().unwrap())).unwrap();
    assert_eq!(task_before.due_at, task_after.due_at);
    assert_eq!(task_before.status, task_after.status);

    // Quiet hours hold non-critical alerts.
    p.quiet_hours_enabled = true;
    p.quiet_start = "00:00".into();
    p.quiet_end = "23:59".into();
    a.save_prefs(&p).unwrap();
    let p = a.prefs().unwrap();
    a.db(|c| {
        c.execute("UPDATE notifications SET state='pending', delivered_at=NULL WHERE category <> 'critical'", [])?;
        c.execute("UPDATE notification_rules SET bypass_quiet=0", [])?;
        Ok(())
    }).unwrap();
    let quiet = a.db(|c| notify::due_toasts(c, &p, now + Duration::minutes(2))).unwrap();
    assert!(quiet.is_empty(), "nothing delivered during quiet hours");
    let held: i64 = a.db(|c| Ok(c.query_row("SELECT COUNT(*) FROM notifications WHERE state='held'", [], |r| r.get(0))?)).unwrap();
    assert!(held > 0, "held alerts are kept, not dropped");
}

#[tokio::test]
async fn closing_a_task_expires_its_pending_reminders() {
    let a = sample_agent().await;
    let p = a.prefs().unwrap();
    a.db(|c| notify::generate(c, &p, Utc::now())).unwrap();
    let t = find(&a, "market sizing");
    a.db(|c| tasks::set_status(c, &t.id, status::COMPLETED, "user", "", None)).unwrap();
    let pending: i64 = a.db(|c| Ok(c.query_row("SELECT COUNT(*) FROM notifications WHERE entity_id=?1 AND state IN ('pending','held')", [&t.id], |r| r.get(0))?)).unwrap();
    assert_eq!(pending, 0);
}

#[tokio::test]
async fn sleep_gap_skips_stale_stages() {
    // A task due in 90 minutes, first seen now (as if the laptop just woke up):
    // we expect a single "due soon" alert, not the missed "coming up tomorrow" one as well.
    let a = Agent::in_memory(Arc::new(MemoryStore::default())).unwrap();
    let mut p = a.prefs().unwrap();
    p.quiet_hours_enabled = false;
    a.save_prefs(&p).unwrap();
    let id = a.db(|c| tasks::create(c, &tasks::NewTask { title: "Submit grant report".into(), due_at: Some(Utc::now() + Duration::minutes(90)), priority: Some(2), ..Default::default() }, "user")).unwrap();
    a.db(|c| planner::reprioritize_all(c, &p)).unwrap();
    a.db(|c| notify::generate(c, &p, Utc::now())).unwrap();
    let ns: Vec<_> = a.db(|c| notify::list(c, "pending", 50)).unwrap().into_iter().filter(|n| n.entity_id.as_deref() == Some(&id)).collect();
    assert_eq!(ns.len(), 1, "{ns:#?}");
}

#[tokio::test]
async fn state_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemoryStore::default());
    {
        let a = Agent::open(dir.path(), secrets.clone()).unwrap();
        a.load_sample().unwrap();
        a.process_pending(200).await.unwrap();
        let p = a.prefs().unwrap();
        a.db(|c| notify::generate(c, &p, Utc::now())).unwrap();
    }
    let a = Agent::open(dir.path(), secrets).unwrap();
    assert!(all(&a).len() >= 10);
    let n = a.db(|c| notify::list(c, "pending", 100)).unwrap().len();
    assert!(n > 0, "pending notifications persisted");
    let (processed, _) = a.process_pending(200).await.unwrap();
    assert_eq!(processed, 0, "nothing re-processed after restart");
}

#[tokio::test]
async fn export_disconnect_and_delete_all() {
    let a = sample_agent().await;
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("export.json");
    a.export_json(&f).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
    assert!(v["tasks"].as_array().unwrap().len() >= 10);
    assert!(!String::from_utf8_lossy(&std::fs::read(&f).unwrap()).contains("refresh_token"));
    let acct = a.accounts().unwrap().into_iter().find(|x| x.provider == "sample").unwrap();
    a.disconnect(&acct.id, true).await.unwrap();
    let items: i64 = a.db(|c| Ok(c.query_row("SELECT COUNT(*) FROM source_items", [], |r| r.get(0))?)).unwrap();
    assert_eq!(items, 0);
    assert_eq!(all(&a).len(), 0, "sample tasks removed with the sample workspace");
    a.load_sample().unwrap();
    a.delete_all_data().unwrap();
    let left: i64 = a.db(|c| Ok(c.query_row("SELECT (SELECT COUNT(*) FROM tasks)+(SELECT COUNT(*) FROM source_items)+(SELECT COUNT(*) FROM accounts)", [], |r| r.get(0))?)).unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn dashboard_and_day_plan() {
    let a = sample_agent().await;
    let p = a.prefs().unwrap();
    let d = a.db(|c| planner::dashboard(c, &p)).unwrap();
    assert!(!d.briefing.is_empty());
    assert!(d.review_count >= 1);
    assert!(!d.overdue.is_empty(), "the market sizing report is overdue");
    let plan = a.db(|c| planner::day_plan(c, &p, Utc::now())).unwrap();
    assert!(plan.blocks.iter().all(|b| b.end > b.start));
}

#[test]
fn patch_null_clears_but_absent_keeps() {
    let p: TaskPatch = serde_json::from_str(r#"{"due_at": null}"#).unwrap();
    assert_eq!(p.due_at, Some(None));
    let p: TaskPatch = serde_json::from_str(r#"{"title": "x"}"#).unwrap();
    assert_eq!(p.due_at, None);
}

#[test]
fn deadline_statement_without_request_is_detected() {
    use kept_core::models::SourceItem;
    let mut p = kept_core::prefs::Preferences::default();
    p.user_name = "Mo Hassan".into();
    p.user_emails = vec!["dr.mwhassan@gmail.com".into()];
    p.timezone = "Asia/Karachi".into();
    let it = SourceItem { id: "x".into(), account_id: "a".into(), provider: "google".into(), kind: "email".into(), external_id: "x".into(), thread_key: None,
        sender_name: Some("Waseem Rehman".into()), sender_email: Some("waseem@example.com".into()), to: vec!["dr.mwhassan@gmail.com".into()], cc: vec![],
        subject: Some("Deadline".into()), body_text: Some("Tomorrow is the deadline, if you fail to deliver you will be fired.".into()), snippet: None, url: None,
        sent_at: Utc::now(), is_from_me: false, process_state: "pending".into(), process_note: None };
    let items = kept_core::extract::rules::extract_one(&p, &it, &[]);
    println!("{items:#?}");
    let t = items.iter().find(|x| kept_core::extract::kind::creates_task(&x.kind)).expect("a task should be detected");
    assert!(t.due.is_some(), "tomorrow should be the deadline");
    assert!(t.urgent);
}

#[tokio::test]
async fn reanalyse_only_touches_unproductive_messages() {
    let a = sample_agent().await;
    let before = all(&a).len();
    let n = a.reanalyse(30).unwrap();
    assert!(n > 0, "negatives (no task) are eligible");
    a.process_pending(200).await.unwrap();
    assert_eq!(all(&a).len(), before, "no duplicates and nothing resurrected");
}
