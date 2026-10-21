//! Ingestion: store normalised provider data, then turn extractor output into
//! tasks, updates, evidence and waiting-for commitments — with dedup and linking.

use crate::db::{audit, new_id, now, opt_ts, ts};
use crate::extract::{kind, ExtractContext, Extracted};
use crate::models::{status, IncomingDocument, IncomingEvent, IncomingExternalTask, IncomingMessage, Person, SourceItem};
use crate::prefs::Preferences;
use crate::tasks::{self, DeadlineOutcome, NewTask, SourceLink};
use crate::{dates, planner, text};
use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};

/// Bodies are capped; we only need the newly written text of a message.
pub const MAX_BODY_CHARS: usize = 12_000;

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct StoreStats {
    pub new_items: usize,
    pub updated_items: usize,
    pub skipped: usize,
}

pub fn store_messages(conn: &Connection, account_id: &str, provider: &str, prefs: &Preferences, msgs: &[IncomingMessage]) -> Result<StoreStats> {
    let mut st = StoreStats::default();
    crate::db::atomic(conn, |tx| {
    for m in msgs {
        let body = text::truncate(&m.body_text, MAX_BODY_CHARS);
        let hash = text::sha256_hex(&format!("{}|{}", m.subject.clone().unwrap_or_default(), body));
        let from_me = m.sender_email.as_deref().map(|e| prefs.is_me(e)).unwrap_or(false);
        let existing: Option<(String, String)> = tx
            .query_row("SELECT id, content_hash FROM source_items WHERE account_id=?1 AND external_id=?2", params![account_id, m.external_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        match existing {
            Some((_, h)) if h == hash => st.skipped += 1, // same message seen again in another sync cycle
            Some((id, _)) => {
                // Edited message (common in chat): re-process.
                tx.execute(
                    "UPDATE source_items SET body_text=?1, snippet=?2, subject=?3, content_hash=?4, process_state='pending' WHERE id=?5",
                    params![body, text::truncate(&text::collapse_ws(&body), 200), m.subject, hash, id],
                )?;
                st.updated_items += 1;
            }
            None => {
                tx.execute(
                    "INSERT INTO source_items(id, account_id, provider, kind, external_id, thread_key, sender_name, sender_email, to_json, cc_json,
                        subject, body_text, snippet, url, sent_at, is_from_me, content_hash, process_state, created_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,'pending',?18)",
                    params![
                        new_id(), account_id, provider, if m.kind.is_empty() { "email" } else { &m.kind }, m.external_id, m.thread_key,
                        m.sender_name, m.sender_email.as_ref().map(|e| e.to_lowercase()), serde_json::to_string(&m.to)?, serde_json::to_string(&m.cc)?,
                        m.subject, body, text::truncate(&text::collapse_ws(&body), 200), m.url, ts(&m.sent_at), from_me, hash, ts(&now())
                    ],
                )?;
                st.new_items += 1;
            }
        }
    }
    tx.execute("UPDATE accounts SET items_synced = items_synced + ?1 WHERE id=?2", params![st.new_items as i64, account_id])?;
    Ok(())
    })?;
    Ok(st)
}

pub fn store_events(conn: &Connection, account_id: &str, events: &[IncomingEvent]) -> Result<StoreStats> {
    let mut st = StoreStats::default();
    crate::db::atomic(conn, |tx| {
    for e in events {
        let existing: Option<String> = tx.query_row("SELECT id FROM meetings WHERE account_id=?1 AND external_id=?2", params![account_id, e.external_id], |r| r.get(0)).optional()?;
        let id = existing.clone().unwrap_or_else(new_id);
        let desc = e.description.as_ref().map(|d| text::truncate(d, 4000));
        if existing.is_some() {
            tx.execute(
                "UPDATE meetings SET title=?1, description=?2, location=?3, start_at=?4, end_at=?5, timezone=?6, all_day=?7, organizer_email=?8,
                    organizer_name=?9, join_url=?10, url=?11, status=?12, my_response=?13, updated_at=?14 WHERE id=?15",
                params![e.title, desc, e.location, ts(&e.start_at), ts(&e.end_at), e.timezone, e.all_day, e.organizer_email, e.organizer_name, e.join_url, e.url, e.status, e.my_response, ts(&now()), id],
            )?;
            st.updated_items += 1;
        } else {
            tx.execute(
                "INSERT INTO meetings(id, account_id, external_id, title, description, location, start_at, end_at, timezone, all_day, organizer_email,
                    organizer_name, join_url, url, status, my_response, updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                params![id, account_id, e.external_id, e.title, desc, e.location, ts(&e.start_at), ts(&e.end_at), e.timezone, e.all_day, e.organizer_email, e.organizer_name, e.join_url, e.url, if e.status.is_empty() { "confirmed" } else { &e.status }, e.my_response, ts(&now())],
            )?;
            st.new_items += 1;
        }
        tx.execute("DELETE FROM meeting_attendees WHERE meeting_id=?1", [&id])?;
        for a in &e.attendees {
            tx.execute(
                "INSERT OR REPLACE INTO meeting_attendees(meeting_id, email, name, response, optional) VALUES (?1,?2,?3,?4,?5)",
                params![id, a.email.to_lowercase(), a.name, a.response, a.optional],
            )?;
        }
        if e.status == "cancelled" {
            tx.execute(
                "UPDATE tasks SET needs_confirmation=1, review_reason='The linked meeting was cancelled' WHERE meeting_id=?1 AND task_type='prep' AND status NOT IN ('completed','cancelled','rejected')",
                [&id],
            )?;
        }
    }
    Ok(())
    })?;
    Ok(st)
}

fn item_row(r: &rusqlite::Row) -> rusqlite::Result<SourceItem> {
    let to: String = r.get(8)?;
    let cc: String = r.get(9)?;
    Ok(SourceItem {
        id: r.get(0)?,
        account_id: r.get(1)?,
        provider: r.get(2)?,
        kind: r.get(3)?,
        external_id: r.get(4)?,
        thread_key: r.get(5)?,
        sender_name: r.get(6)?,
        sender_email: r.get(7)?,
        to: serde_json::from_str(&to).unwrap_or_default(),
        cc: serde_json::from_str(&cc).unwrap_or_default(),
        subject: r.get(10)?,
        body_text: r.get(11)?,
        snippet: r.get(12)?,
        url: r.get(13)?,
        sent_at: opt_ts(r.get(14)?).unwrap_or_else(Utc::now),
        is_from_me: r.get::<_, i64>(15)? != 0,
        process_state: r.get(16)?,
        process_note: r.get(17)?,
    })
}

const ITEM_COLS: &str = "id, account_id, provider, kind, external_id, thread_key, sender_name, sender_email, to_json, cc_json, subject, body_text, snippet, url, sent_at, is_from_me, process_state, process_note";

pub fn get_item(conn: &Connection, id: &str) -> Result<Option<SourceItem>> {
    Ok(conn.query_row(&format!("SELECT {ITEM_COLS} FROM source_items WHERE id=?1"), [id], item_row).optional()?)
}

pub fn list_items(conn: &Connection, limit: i64) -> Result<Vec<SourceItem>> {
    let mut st = conn.prepare(&format!("SELECT {ITEM_COLS} FROM source_items ORDER BY sent_at DESC LIMIT ?1"))?;
    let v = st.query_map([limit], item_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(v)
}

/// One unit of extraction work, prepared under the DB lock and processed without it.
pub struct Prepared {
    pub item: SourceItem,
    pub thread: Vec<SourceItem>,
    pub open_tasks: Vec<(String, String)>,
}

pub fn prepare_batch(conn: &Connection, limit: i64) -> Result<Vec<Prepared>> {
    let mut st = conn.prepare(&format!("SELECT {ITEM_COLS} FROM source_items WHERE process_state='pending' ORDER BY sent_at ASC LIMIT ?1"))?;
    let items = st.query_map([limit], item_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    let open: Vec<(String, String)> = {
        let mut st = conn.prepare("SELECT id, title FROM tasks WHERE status NOT IN ('completed','cancelled','rejected') ORDER BY updated_at DESC LIMIT 200")?;
        let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        v
    };
    let mut out = Vec::new();
    for item in items {
        let thread = match &item.thread_key {
            Some(k) => {
                let mut st = conn.prepare(&format!("SELECT {ITEM_COLS} FROM source_items WHERE thread_key=?1 AND sent_at < ?2 AND id <> ?3 ORDER BY sent_at DESC LIMIT 6"))?;
                let mut v = st.query_map(params![k, ts(&item.sent_at), item.id], item_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
                v.reverse();
                v
            }
            None => vec![],
        };
        // Offer the extractor only tasks plausibly related (keeps prompts small).
        let subj = format!("{} {}", item.subject.clone().unwrap_or_default(), item.snippet.clone().unwrap_or_default());
        let mut related: Vec<(String, String)> = open.iter().filter(|(_, t)| text::overlap(t, &subj) >= 0.3).take(15).cloned().collect();
        if let Some(k) = &item.thread_key {
            let mut st = conn.prepare("SELECT DISTINCT t.id, t.title FROM tasks t JOIN task_sources ts ON ts.task_id=t.id JOIN source_items si ON si.id=ts.source_item_id WHERE si.thread_key=?1 AND t.status NOT IN ('cancelled','rejected')")?;
            for r in st.query_map([k], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let r = r?;
                if !related.iter().any(|x| x.0 == r.0) {
                    related.push(r);
                }
            }
        }
        out.push(Prepared { item, thread, open_tasks: related });
    }
    Ok(out)
}

pub fn mark_processed(conn: &Connection, item_id: &str, state: &str, note: &str) -> Result<()> {
    conn.execute("UPDATE source_items SET process_state=?1, process_note=?2, processed_at=?3 WHERE id=?4", params![state, note, ts(&now()), item_id])?;
    Ok(())
}

fn label_for(item: &SourceItem) -> String {
    let who = item.sender_name.clone().or_else(|| item.sender_email.clone()).unwrap_or_else(|| "unknown".into());
    let what = match item.kind.as_str() {
        "chat" => "Message",
        "document" => "Document",
        _ => "Email",
    };
    let dir = if item.is_from_me { format!("{what} you sent") } else { format!("{what} from {who}") };
    format!("{dir} · {}", item.sent_at.format("%d %b %Y"))
}

fn add_item_source(conn: &Connection, task_id: &str, item: &SourceItem, relation: &str, quote: &str) -> Result<()> {
    tasks::add_source(
        conn,
        task_id,
        &SourceLink {
            source_item_id: Some(&item.id),
            meeting_id: None,
            relation,
            quote: Some(quote),
            provider: Some(&item.provider),
            url: item.url.as_deref(),
            label: Some(label_for(item)),
        },
    )
}

/// Find the meeting a "before the meeting" deadline refers to.
/// Rules: a named weekday must match; a descriptor ("standup", "project review") must match
/// the meeting title; with neither, at least two shared participants within a week.
pub fn find_meeting_for(conn: &Connection, item: &SourceItem, guess: &dates::DeadlineGuess, _title: &str, tz: chrono_tz::Tz) -> Result<Option<(String, DateTime<Utc>, String)>> {
    let from = item.sent_at;
    let horizon = if guess.meeting_weekday.is_some() || guess.meeting_hint.is_some() { 14 } else { 7 };
    let to = from + Duration::days(horizon);
    let mut st = conn.prepare("SELECT id, title, start_at FROM meetings WHERE start_at > ?1 AND start_at < ?2 AND status <> 'cancelled' ORDER BY start_at")?;
    let rows = st.query_map(params![ts(&from), ts(&to)], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let mut people: Vec<String> = item.to.iter().chain(item.cc.iter()).map(|a| text::parse_address(a).1).collect();
    if let Some(s) = &item.sender_email {
        people.push(s.clone());
    }
    let mut best: Option<(i64, String, DateTime<Utc>, String)> = None;
    for r in rows {
        let (id, mt, start) = r?;
        let Some(start) = crate::db::parse_ts(&start) else { continue };
        if let Some(wd) = guess.meeting_weekday {
            if start.with_timezone(&tz).weekday() != wd {
                continue;
            }
        }
        let mut shared = 0i64;
        for p in &people {
            let hit: Option<i64> = conn.query_row("SELECT 1 FROM meeting_attendees WHERE meeting_id=?1 AND email=?2", params![id, p], |r| r.get(0)).optional()?;
            if hit.is_some() {
                shared += 1;
            }
        }
        let hint_ok = guess.meeting_hint.as_deref().map(|h| text::overlap(h, &mt) >= 0.5 || mt.to_lowercase().contains(&h.to_lowercase()));
        let ok = match (guess.meeting_weekday.is_some(), hint_ok) {
            (_, Some(false)) => false,
            (_, Some(true)) => true,
            (true, None) => true,
            (false, None) => shared >= 2,
        };
        if !ok {
            continue;
        }
        // Earlier and more shared participants win.
        let score = shared * 10 - (start - from).num_hours() / 24;
        if best.as_ref().map(|b| score > b.0).unwrap_or(true) {
            best = Some((score, id, start, mt));
        }
    }
    Ok(best.map(|b| (b.1, b.2, b.3)))
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ApplyStats {
    pub created: usize,
    pub updated: usize,
    pub for_review: usize,
    pub completed: usize,
    pub waiting_for: usize,
    pub ignored: usize,
}

/// Turn extracted items for one message into durable state.
pub fn apply(conn: &Connection, prefs: &Preferences, item: &SourceItem, items: &[Extracted]) -> Result<ApplyStats> {
    let mut stats = ApplyStats::default();
    let tz = prefs.tz();
    let wd = crate::extract::rules::workday(prefs);
    crate::db::atomic(conn, |tx| {
    for it in items {
        match it.kind.as_str() {
            k if kind::creates_task(k) && k != kind::FOLLOWUP_NUDGE => {
                apply_task(&tx, prefs, item, it, &mut stats, tz, wd)?;
            }
            kind::FOLLOWUP_NUDGE => apply_nudge(&tx, prefs, item, it, &mut stats, tz, wd)?,
            kind::THEIR_COMMITMENT => {
                let who = item.sender_name.clone();
                tx.execute(
                    "INSERT INTO commitments(id, source_item_id, direction, who_name, who_email, text, due_at, status, created_at) VALUES (?1,?2,'theirs',?3,?4,?5,?6,'open',?7)",
                    params![new_id(), item.id, who, item.sender_email, it.title, it.due.as_ref().map(|d| ts(&d.due_at)), ts(&now())],
                )?;
                stats.waiting_for += 1;
            }
            kind::COMPLETION_EVIDENCE => apply_completion(&tx, prefs, item, it, &mut stats)?,
            kind::APPROVAL_RECEIVED => apply_approval(&tx, item, it, &mut stats)?,
            _ => stats.ignored += 1,
        }
    }
    Ok(())
    })?;
    Ok(stats)
}

fn apply_task(conn: &Connection, prefs: &Preferences, item: &SourceItem, it: &Extracted, stats: &mut ApplyStats, tz: chrono_tz::Tz, wd: dates::WorkDay) -> Result<()> {
    // Resolve meeting-relative deadlines against the calendar.
    let mut due = it.due.clone();
    let mut meeting_id = None;
    let mut meeting_note = None;
    if let Some(g) = &it.due {
        if g.before_meeting {
            if let Some((mid, start, mt)) = find_meeting_for(conn, item, g, &it.title, tz)? {
                let d = start - Duration::minutes(prefs.meeting_prep_buffer_minutes.max(0));
                due = Some(dates::DeadlineGuess { due_at: d, kind: "explicit".into(), ..g.clone() });
                meeting_id = Some(mid);
                meeting_note = Some(format!("Due before \"{mt}\""));
            } else {
                meeting_note = Some("Mentions a meeting Kept could not find in your calendar — confirm the deadline.".into());
                due = due.map(|mut d| {
                    // Meetings are usually during the day: aim for the start of that working day.
                    let day = d.due_at.with_timezone(&tz).date_naive();
                    d.due_at = dates::local_to_utc(tz, day, wd.start);
                    d.kind = "inferred".into();
                    d
                });
            }
        }
    }
    let counterpart_email = it.counterpart.as_ref().and_then(|c| c.email.clone());
    let thread = item.thread_key.as_deref();

    // Update an existing task instead of duplicating it.
    let target = match &it.related_task_id {
        Some(id) => Some((id.clone(), 1.0, true)),
        None => tasks::find_duplicate(conn, &it.title, thread, counterpart_email.as_deref(), None)?,
    };
    if let Some((tid, _score, strong)) = &target {
        if *strong {
            add_item_source(conn, tid, item, "update", &it.evidence)?;
            if let Some(d) = &due {
                let outcome = tasks::apply_deadline(conn, tid, &d.kind, d.due_at, Some(&d.phrase), Some(&item.id), item.sent_at, !item.is_from_me)?;
                if let DeadlineOutcome::Changed { .. } = outcome {
                    audit(conn, "task", tid, "deadline_source", None, None, "agent", &format!("from {}", label_for(item)))?;
                }
            }
            if let Some(m) = &meeting_id {
                conn.execute("UPDATE tasks SET meeting_id=COALESCE(meeting_id, ?1) WHERE id=?2", params![m, tid])?;
            }
            conn.execute("UPDATE tasks SET last_update_at=?1, updated_at=?1 WHERE id=?2", params![ts(&now()), tid])?;
            stats.updated += 1;
            return Ok(());
        }
    }

    let low_conf = it.confidence < prefs.auto_accept_threshold;
    let needs_review = it.needs_confirmation || low_conf || prefs.review_everything || it.my_role != "owner" || target.is_some();
    let review_reason = if let Some(a) = &it.ambiguity {
        Some(a.clone())
    } else if target.is_some() {
        Some("Possible duplicate of an existing task".into())
    } else if it.my_role != "owner" {
        Some(format!("You may be a {} rather than the owner", it.my_role))
    } else if low_conf {
        Some(format!("Detected with {:.0}% confidence", it.confidence * 100.0))
    } else if prefs.review_everything {
        Some("You asked to review every detected task".into())
    } else {
        None
    };
    let mut people = vec![];
    if let Some(c) = &it.counterpart {
        people.push(c.clone());
    }
    let effort = planner::estimate_effort(&it.task_type, &it.title);
    let description = match (&it.description, &meeting_note) {
        (Some(d), Some(m)) => Some(format!("{d}\n{m}")),
        (Some(d), None) => Some(d.clone()),
        (None, Some(m)) => Some(m.clone()),
        _ => None,
    };
    let new = NewTask {
        title: it.title.clone(),
        description,
        task_type: it.task_type.clone(),
        my_role: it.my_role.clone(),
        owner_name: it.owner_name.clone(),
        status: if needs_review { status::NEEDS_CONFIRMATION.into() } else { status::READY.into() },
        priority: None,
        confidence: it.confidence,
        origin: it.extractor.clone(),
        needs_confirmation: needs_review,
        review_reason,
        due_at: due.as_ref().map(|d| d.due_at),
        due_kind: due.as_ref().map(|d| d.kind.clone()).unwrap_or_default(),
        due_phrase: due.as_ref().map(|d| d.phrase.clone()),
        committed_at: Some(item.sent_at),
        meeting_id: meeting_id.clone(),
        project: it.project.clone(),
        expected_deliverable: it.expected_deliverable.clone(),
        completion_criteria: it.completion_criteria.clone(),
        effort_minutes: Some(effort),
        people,
        counterpart_email,
        possible_duplicate_of: target.as_ref().map(|t| t.0.clone()),
        ..Default::default()
    };
    let id = tasks::create(conn, &new, &it.extractor)?;
    add_item_source(conn, &id, item, "origin", &it.evidence)?;
    if due.is_none() && matches!(it.kind.as_str(), kind::MY_COMMITMENT | kind::REQUEST_TO_ME | kind::APPROVAL_REQUEST) {
        let s = dates::suggest(&it.task_type, item.sent_at, tz, wd);
        tasks::apply_deadline(conn, &id, "suggested", s, None, Some(&item.id), item.sent_at, false)?;
    }
    let direction = if it.kind == kind::MY_COMMITMENT { "mine" } else { "request_to_me" };
    conn.execute(
        "INSERT INTO commitments(id, source_item_id, direction, who_name, who_email, text, due_at, status, task_id, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,'open',?8,?9)",
        params![
            new_id(), item.id, direction,
            it.counterpart.as_ref().and_then(|c| c.name.clone()), it.counterpart.as_ref().and_then(|c| c.email.clone()),
            it.evidence, due.as_ref().map(|d| ts(&d.due_at)), id, ts(&now())
        ],
    )?;
    planner::reprioritize_one(conn, prefs, &id)?;
    if it.urgent {
        conn.execute("UPDATE tasks SET priority=MAX(1, priority-1), priority_reason=COALESCE(priority_reason,'') || ' · marked urgent' WHERE id=?1 AND priority_locked=0", [&id])?;
    }
    if needs_review {
        stats.for_review += 1;
    } else {
        stats.created += 1;
    }
    Ok(())
}

fn apply_nudge(conn: &Connection, prefs: &Preferences, item: &SourceItem, it: &Extracted, stats: &mut ApplyStats, tz: chrono_tz::Tz, wd: dates::WorkDay) -> Result<()> {
    // Look for the original commitment: same thread first, then similar tasks owed to this person.
    let sender = item.sender_email.clone().unwrap_or_default();
    let mut st = conn.prepare(
        "SELECT t.id, t.title, t.status, t.counterpart_email,
            EXISTS(SELECT 1 FROM task_sources ts JOIN source_items si ON si.id=ts.source_item_id WHERE ts.task_id=t.id AND si.thread_key=?1)
         FROM tasks t WHERE t.status NOT IN ('rejected','cancelled') AND t.duplicate_of IS NULL",
    )?;
    let rows = st.query_map([item.thread_key.clone().unwrap_or_default()], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, bool>(4)?))
    })?;
    let probe = format!("{} {} {}", it.title, it.expected_deliverable.clone().unwrap_or_default(), item.subject.clone().unwrap_or_default());
    let mut best: Option<(f64, String, String)> = None;
    for r in rows {
        let (id, title, st, cp, same_thread) = r?;
        if it.related_task_id.as_deref() == Some(id.as_str()) {
            best = Some((10.0, id, st));
            break;
        }
        let mut score = text::overlap(&title, &probe);
        if same_thread {
            score += 0.6;
        }
        if cp.as_deref().map(|c| c.eq_ignore_ascii_case(&sender)).unwrap_or(false) {
            score += 0.3;
        }
        if score >= 0.6 && best.as_ref().map(|b| score > b.0).unwrap_or(true) {
            best = Some((score, id, st));
        }
    }
    let who = item.sender_name.clone().unwrap_or_else(|| sender.clone());
    if let Some((_, tid, st)) = best {
        add_item_source(conn, &tid, item, "followup", &it.evidence)?;
        if status::is_open(&st) {
            conn.execute(
                "UPDATE tasks SET priority = CASE WHEN priority_locked=1 THEN priority ELSE MAX(1, priority-1) END,
                    priority_reason = CASE WHEN priority_locked=1 THEN priority_reason ELSE ?1 END, last_update_at=?2, updated_at=?2 WHERE id=?3",
                params![format!("{who} followed up on this"), ts(&now()), tid],
            )?;
            audit(conn, "task", &tid, "nudged", None, None, "agent", &format!("{who} followed up"))?;
        } else {
            // Marked done, yet someone is still chasing it: ask the user.
            conn.execute(
                "UPDATE tasks SET status='needs_confirmation', needs_confirmation=1, completed_at=NULL,
                    review_reason=?1, updated_at=?2 WHERE id=?3",
                params![format!("Marked {st}, but {who} is still following up — was it actually delivered?"), ts(&now()), tid],
            )?;
            audit(conn, "task", &tid, "status", Some(&st), Some("needs_confirmation"), "agent", &format!("reopened: {who} followed up"))?;
            stats.for_review += 1;
            return Ok(());
        }
        stats.updated += 1;
        return Ok(());
    }
    // No original found — create a follow-up task, flagged for review.
    let mut x = it.clone();
    x.needs_confirmation = true;
    x.ambiguity = Some(format!("{who} says you promised something, but Kept found no matching commitment in synced data."));
    apply_task(conn, prefs, item, &x, stats, tz, wd)
}

fn apply_completion(conn: &Connection, prefs: &Preferences, item: &SourceItem, it: &Extracted, stats: &mut ApplyStats) -> Result<()> {
    let recipients: Vec<String> = item.to.iter().map(|a| text::parse_address(a).1).collect();
    let mut st = conn.prepare(
        "SELECT t.id, t.title, t.status, t.counterpart_email, t.completion_criteria, t.task_type, t.expected_deliverable,
            EXISTS(SELECT 1 FROM task_sources ts JOIN source_items si ON si.id=ts.source_item_id WHERE ts.task_id=t.id AND si.thread_key=?1)
         FROM tasks t WHERE t.status NOT IN ('completed','cancelled','rejected') AND t.my_role IN ('owner','collaborator') AND t.duplicate_of IS NULL",
    )?;
    let rows = st.query_map([item.thread_key.clone().unwrap_or_default()], |r| {
        Ok((
            r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?, r.get::<_, String>(5)?, r.get::<_, Option<String>>(6)?, r.get::<_, bool>(7)?,
        ))
    })?;
    let probe = format!("{} {} {}", it.evidence, it.expected_deliverable.clone().unwrap_or_default(), item.subject.clone().unwrap_or_default());
    // (score, strong, id, status, criteria, type)
    let mut best: Option<(f64, bool, String, String, Option<String>, String)> = None;
    for r in rows {
        let (id, title, st, cp, crit, ttype, deliv, same_thread) = r?;
        if matches!(ttype.as_str(), "approval" | "reply") && it.completion_stage.as_deref() == Some("draft_sent") {
            continue;
        }
        let ov = text::overlap(&format!("{title} {}", deliv.unwrap_or_default()), &probe);
        let to_cp = cp.as_deref().map(|c| recipients.iter().any(|r| r.eq_ignore_ascii_case(c))).unwrap_or(false);
        let related = it.related_task_id.as_deref() == Some(id.as_str());
        let score = ov + if same_thread { 0.5 } else { 0.0 } + if to_cp { 0.3 } else { 0.0 } + if related { 1.0 } else { 0.0 };
        let strong = related || (same_thread && (ov >= 0.25 || to_cp)) || (to_cp && ov >= 0.5);
        if score >= 0.5 && best.as_ref().map(|b| score > b.0).unwrap_or(true) {
            best = Some((score, strong, id, st, crit, ttype));
        }
    }
    let stage = it.completion_stage.clone().unwrap_or_else(|| "sent".into());
    let to_label = item.to.first().map(|a| text::parse_address(a)).map(|(n, e)| n.unwrap_or(e)).unwrap_or_else(|| "recipient".into());
    let evidence = format!("You {} on {}: \"{}\"", if stage == "draft_sent" { format!("sent a draft to {to_label}") } else { format!("sent it to {to_label}") }, item.sent_at.format("%d %b %Y %H:%M UTC"), text::truncate(&it.evidence, 160));
    match best {
        Some((_, true, tid, cur, crit, _)) => {
            add_item_source(conn, &tid, item, "evidence", &it.evidence)?;
            let needs_approval = crit.as_deref().map(|c| c.to_lowercase().contains("approv")).unwrap_or(false);
            if stage == "draft_sent" || needs_approval {
                // Draft sent ≠ done. Track the approval instead.
                if cur != status::AWAITING_APPROVAL {
                    tasks::set_status(conn, &tid, status::AWAITING_APPROVAL, "agent", &evidence, None)?;
                }
                conn.execute("UPDATE tasks SET completion_evidence=?1 WHERE id=?2", params![format!("{evidence} — awaiting approval, not complete."), tid])?;
            } else {
                tasks::set_status(conn, &tid, status::COMPLETED, "agent", "sent message detected", Some(&evidence))?;
                stats.completed += 1;
            }
            stats.updated += 1;
        }
        Some((_, false, tid, _, _, _)) => {
            add_item_source(conn, &tid, item, "evidence", &it.evidence)?;
            conn.execute(
                "UPDATE tasks SET needs_confirmation=1, review_reason=?1, updated_at=?2 WHERE id=?3",
                params![format!("Possible completion evidence — {evidence}. Confirm whether this finishes the task."), ts(&now()), tid],
            )?;
            stats.for_review += 1;
        }
        None if stage == "draft_sent" => {
            // Sent a draft that we weren't tracking: track the pending approval/feedback.
            let deliverable = it.expected_deliverable.clone().unwrap_or_else(|| item.subject.clone().unwrap_or_else(|| "draft".into()));
            let id = tasks::create(
                conn,
                &NewTask {
                    title: text::truncate(&format!("Get sign-off on {deliverable} from {to_label}"), 100),
                    description: Some("You sent a draft. The final deliverable is not complete until it is reviewed / approved.".into()),
                    task_type: "followup".into(),
                    status: status::AWAITING_APPROVAL.into(),
                    confidence: 0.7,
                    origin: it.extractor.clone(),
                    needs_confirmation: 0.7 < prefs.auto_accept_threshold,
                    review_reason: Some("Created because you sent a draft".into()),
                    committed_at: Some(item.sent_at),
                    expected_deliverable: Some(deliverable),
                    completion_criteria: Some(format!("{to_label} approves the final version")),
                    people: item.to.first().map(|a| { let (n, e) = text::parse_address(a); vec![Person { name: n, email: Some(e) }] }).unwrap_or_default(),
                    counterpart_email: item.to.first().map(|a| text::parse_address(a).1),
                    effort_minutes: Some(10),
                    ..Default::default()
                },
                &it.extractor,
            )?;
            add_item_source(conn, &id, item, "origin", &it.evidence)?;
            stats.created += 1;
        }
        None => stats.ignored += 1,
    }
    Ok(())
}

fn apply_approval(conn: &Connection, item: &SourceItem, it: &Extracted, stats: &mut ApplyStats) -> Result<()> {
    let Some(thread) = &item.thread_key else {
        stats.ignored += 1;
        return Ok(());
    };
    let tid: Option<String> = conn
        .query_row(
            "SELECT t.id FROM tasks t JOIN task_sources ts ON ts.task_id=t.id JOIN source_items si ON si.id=ts.source_item_id
             WHERE si.thread_key=?1 AND t.status IN ('awaiting_approval','sent') ORDER BY t.updated_at DESC LIMIT 1",
            [thread],
            |r| r.get(0),
        )
        .optional()?;
    match tid {
        Some(tid) => {
            add_item_source(conn, &tid, item, "completion", &it.evidence)?;
            let who = item.sender_name.clone().or_else(|| item.sender_email.clone()).unwrap_or_default();
            let ev = format!("Approved by {who} on {}: \"{}\"", item.sent_at.format("%d %b %Y"), text::truncate(&it.evidence, 160));
            tasks::set_status(conn, &tid, status::COMPLETED, "agent", "approval received", Some(&ev))?;
            stats.completed += 1;
        }
        None => stats.ignored += 1,
    }
    Ok(())
}

/// Run the built-in rules extractor over prepared items (used when AI is off or fails).
pub fn extract_rules(prefs: &Preferences, p: &Prepared) -> Vec<Extracted> {
    let ctx = ExtractContext { prefs, item: &p.item, thread: &p.thread, open_tasks: &p.open_tasks, now: now() };
    crate::extract::rules::extract(&ctx)
}

/// Retention: purge bodies and old items per user settings. Tasks keep their source label + quote.
pub fn apply_retention(conn: &Connection, prefs: &Preferences) -> Result<(usize, usize)> {
    let body_cut = ts(&(now() - Duration::days(prefs.retention_body_days as i64)));
    let item_cut = ts(&(now() - Duration::days(prefs.retention_item_days as i64)));
    let a = conn.execute("UPDATE source_items SET body_text=NULL WHERE body_text IS NOT NULL AND sent_at < ?1 AND process_state <> 'pending'", [&body_cut])?;
    let b = conn.execute("DELETE FROM source_items WHERE sent_at < ?1 AND process_state <> 'pending'", [&item_cut])?;
    conn.execute("DELETE FROM meetings WHERE end_at < ?1", [&item_cut])?;
    conn.execute("DELETE FROM notifications WHERE created_at < ?1 AND state IN ('dismissed','expired','delivered','in_app')", [&item_cut])?;
    Ok((a, b))
}



/// Version of the detection rules. Bump when extraction improves, so messages that
/// produced nothing are re-analysed once after an update.
pub const RULES_VERSION: &str = "2026.10.18-3";

/// Mark recent messages that produced no task, update or waiting-for item as pending again.
/// Messages that created anything (including tasks you rejected) are left alone, so nothing
/// you already handled comes back.
pub fn reanalyse_unproductive(conn: &Connection, days: i64) -> Result<usize> {
    let since = ts(&(now() - Duration::days(days.clamp(1, 90))));
    let n = conn.execute(
        "UPDATE source_items SET process_state='pending', process_note='re-analysis requested'
         WHERE sent_at > ?1 AND process_state IN ('done','error') AND body_text IS NOT NULL
           AND NOT EXISTS (SELECT 1 FROM task_sources ts WHERE ts.source_item_id = source_items.id)
           AND NOT EXISTS (SELECT 1 FROM commitments c WHERE c.source_item_id = source_items.id)",
        [since],
    )?;
    Ok(n)
}


pub fn store_documents(conn: &Connection, account_id: &str, provider: &str, docs: &[IncomingDocument]) -> Result<StoreStats> {
    let mut st = StoreStats::default();
    crate::db::atomic(conn, |tx| {
        for d in docs {
            let n = tx.execute(
                "INSERT INTO documents(id, account_id, provider, external_id, name, mime_type, url, modified_at, modified_by, owner, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                 ON CONFLICT(account_id, external_id) DO UPDATE SET name=excluded.name, url=excluded.url, modified_at=excluded.modified_at,
                    modified_by=excluded.modified_by, mime_type=excluded.mime_type, updated_at=excluded.updated_at",
                params![new_id(), account_id, provider, d.external_id, text::truncate(&d.name, 300), d.mime_type, d.url, d.modified_at.as_ref().map(ts), d.modified_by, d.owner, ts(&now())],
            )?;
            st.new_items += n;
        }
        Ok(())
    })?;
    Ok(st)
}

fn provider_label(p: &str) -> &'static str {
    match p {
        "jira" => "Jira",
        "asana" => "Asana",
        "todoist" => "Todoist",
        "linear" => "Linear",
        _ => "task tool",
    }
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct ExternalStats {
    pub created: usize,
    pub linked: usize,
    pub closed: usize,
    pub updated: usize,
    pub flagged: usize,
}

/// Mirror tasks assigned to you in a task tool into local tasks.
/// * new open item → linked to a matching detected task if one exists, otherwise created;
/// * closed in the tool → local task completed, with that as evidence;
/// * no longer in the (complete) list → flagged for review, never silently completed.
pub fn store_external_tasks(conn: &Connection, prefs: &Preferences, account_id: &str, provider: &str, items: &[IncomingExternalTask], snapshot_complete: bool) -> Result<ExternalStats> {
    let mut stats = ExternalStats::default();
    let tz = prefs.tz();
    let wd = crate::extract::rules::workday(prefs);
    let label = provider_label(provider);
    crate::db::atomic(conn, |tx| {
        let mut seen: Vec<String> = vec![];
        for it in items {
            seen.push(it.external_id.clone());
            let due = it.due_at.map(|d| if it.due_is_date_only { dates::local_to_utc(tz, d.date_naive(), wd.end) } else { d });
            let existing: Option<(String, Option<String>, i64)> = tx
                .query_row("SELECT id, task_id, is_done FROM external_tasks WHERE account_id=?1 AND external_id=?2", params![account_id, it.external_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .optional()?;
            let ext_id = existing.as_ref().map(|e| e.0.clone()).unwrap_or_else(new_id);
            let mut task_id = existing.as_ref().and_then(|e| e.1.clone());
            // Local task deleted by the user? Treat as unlinked but don't recreate closed ones.
            if let Some(tid) = &task_id {
                if tx.query_row("SELECT 1 FROM tasks WHERE id=?1", [tid], |_| Ok(())).optional()?.is_none() {
                    task_id = None;
                    if existing.is_some() {
                        // The user deleted it deliberately — respect that.
                        tx.execute("UPDATE external_tasks SET title=?1, status=?2, is_done=?3, updated_at=?4 WHERE id=?5", params![it.title, it.status, it.is_done, ts(&now()), ext_id])?;
                        continue;
                    }
                }
            }
            let source_label = format!("{label}{} · {}", it.key.as_ref().map(|k| format!(" {k}")).unwrap_or_default(), it.project.clone().unwrap_or_else(|| "assigned to you".into()));
            if task_id.is_none() && !it.is_done {
                // Same work already detected from email/chat? Link instead of duplicating.
                // Tool titles are terse ("Book flights to Dubai" vs "Book flights to Dubai for the offsite"):
                // accept a high keyword overlap as the same work.
                let dup = tasks::find_duplicate(tx, &it.title, None, None, None)?.filter(|(id, score, strong)| {
                    (*strong || *score >= 0.8) && tx.query_row("SELECT origin FROM tasks WHERE id=?1", [id], |r| r.get::<_, String>(0)).map(|o| o != "external").unwrap_or(false)
                });
                if let Some((tid, _, _)) = dup {
                    task_id = Some(tid.clone());
                    stats.linked += 1;
                } else {
                    let new = NewTask {
                        title: it.title.clone(),
                        description: it.description.clone(),
                        task_type: "action".into(),
                        my_role: "owner".into(),
                        status: status::READY.into(),
                        priority: None,
                        confidence: 1.0,
                        origin: "external".into(),
                        due_at: due,
                        due_kind: if due.is_some() { "explicit".into() } else { String::new() },
                        due_phrase: due.map(|_| format!("due date in {label}")),
                        project: it.project.clone(),
                        effort_minutes: Some(planner::estimate_effort("action", &it.title)),
                        ..Default::default()
                    };
                    let id = tasks::create(tx, &new, "connector")?;
                    if let Some(p) = it.priority {
                        tx.execute("UPDATE tasks SET priority=?1, priority_reason=?2 WHERE id=?3", params![p, format!("priority set in {label}"), id])?;
                    }
                    task_id = Some(id);
                    stats.created += 1;
                }
                if let Some(tid) = &task_id {
                    tasks::add_source(tx, tid, &SourceLink { source_item_id: None, meeting_id: None, relation: "external", quote: Some(&it.title), provider: Some(provider), url: it.url.as_deref(), label: Some(source_label.clone()) })?;
                }
            } else if let Some(tid) = &task_id {
                let cur = tasks::get(tx, tid)?;
                if it.is_done && status::is_open(&cur.status) {
                    let ev = format!("Marked {} in {label}{} on {}", it.status.clone().unwrap_or_else(|| "done".into()), it.key.as_ref().map(|k| format!(" ({k})")).unwrap_or_default(), now().format("%d %b %Y"));
                    tasks::set_status(tx, tid, status::COMPLETED, "connector", "closed in task tool", Some(&ev))?;
                    stats.closed += 1;
                } else if !it.is_done {
                    if let Some(d) = due {
                        if let tasks::DeadlineOutcome::Changed { .. } = tasks::apply_deadline(tx, tid, "explicit", d, Some(&format!("due date in {label}")), None, now(), true)? {
                            stats.updated += 1;
                        }
                    }
                    if cur.origin == "external" && cur.title != it.title {
                        tx.execute("UPDATE tasks SET title=?1, fingerprint=?2, updated_at=?3 WHERE id=?4", params![it.title, text::fingerprint(&it.title), ts(&now()), tid])?;
                        stats.updated += 1;
                    }
                }
            }
            tx.execute(
                "INSERT INTO external_tasks(id, account_id, provider, external_id, key, title, status, is_done, due_at, url, project, task_id, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
                 ON CONFLICT(account_id, external_id) DO UPDATE SET key=excluded.key, title=excluded.title, status=excluded.status, is_done=excluded.is_done,
                    due_at=excluded.due_at, url=excluded.url, project=excluded.project, task_id=COALESCE(excluded.task_id, external_tasks.task_id), updated_at=excluded.updated_at",
                params![ext_id, account_id, provider, it.external_id, it.key, it.title, it.status, it.is_done, due.as_ref().map(ts), it.url, it.project, task_id, ts(&now())],
            )?;
        }
        if snapshot_complete {
            // Mirrored items that vanished: reassigned, deleted, or completed long ago. Ask, don't assume.
            let mut st = tx.prepare("SELECT e.id, e.external_id, e.task_id, e.key FROM external_tasks e WHERE e.account_id=?1 AND e.is_done=0")?;
            let gone: Vec<(String, String, Option<String>, Option<String>)> = st.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
            drop(st);
            for (eid, xid, tid, key) in gone {
                if seen.contains(&xid) {
                    continue;
                }
                tx.execute("UPDATE external_tasks SET is_done=1, status='no longer open or assigned', updated_at=?1 WHERE id=?2", params![ts(&now()), eid])?;
                if let Some(tid) = tid {
                    tx.execute(
                        "UPDATE tasks SET needs_confirmation=1, review_reason=?1, updated_at=?2 WHERE id=?3 AND status NOT IN ('completed','cancelled','rejected')",
                        params![format!("No longer open or assigned to you in {label}{} — completed, reassigned or deleted? Confirm.", key.map(|k| format!(" ({k})")).unwrap_or_default()), ts(&now()), tid],
                    )?;
                    stats.flagged += 1;
                }
            }
        }
        Ok(())
    })?;
    Ok(stats)
}
