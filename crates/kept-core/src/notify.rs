//! Notification engine: decides *what* deserves an alert and *when*.
//!
//! Anti-spam design:
//! * Every candidate has a stable fingerprint (kind + entity + stage). The `notifications`
//!   table has a UNIQUE index on it, so re-syncing the same email can never re-alert.
//! * Only the *current* stage is generated. After sleep/offline, missed earlier stages are
//!   skipped (e.g. you get "due in 2 h", not "due tomorrow" + "due in 2 h").
//! * Quiet hours hold non-critical alerts; a per-hour rate limit and grouping consolidate bursts.
//! * Snooze / dismiss affect only the notification, never the task or its deadline.

use crate::db::{new_id, now, opt_ts, parse_ts, ts};
use crate::models::{Notification, NotificationRule, Toast};
use crate::prefs::{self, parse_hm, Preferences};
use crate::{meetings, planner, text};
use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, NaiveTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

pub fn rules(conn: &Connection) -> Result<Vec<NotificationRule>> {
    let mut st = conn.prepare("SELECT category, enabled, desktop, lead_minutes, repeat_minutes, min_priority, bypass_quiet FROM notification_rules ORDER BY category")?;
    let v = st
        .query_map([], |r| {
            Ok(NotificationRule {
                category: r.get(0)?,
                enabled: r.get::<_, i64>(1)? != 0,
                desktop: r.get::<_, i64>(2)? != 0,
                lead_minutes: r.get(3)?,
                repeat_minutes: r.get(4)?,
                min_priority: r.get(5)?,
                bypass_quiet: r.get::<_, i64>(6)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(v)
}

pub fn save_rule(conn: &Connection, r: &NotificationRule) -> Result<()> {
    conn.execute(
        "UPDATE notification_rules SET enabled=?1, desktop=?2, lead_minutes=?3, repeat_minutes=?4, min_priority=?5, bypass_quiet=?6 WHERE category=?7",
        params![r.enabled, r.desktop, r.lead_minutes.clamp(0, 20160), r.repeat_minutes.clamp(0, 10080), r.min_priority.clamp(1, 4), r.bypass_quiet, r.category],
    )?;
    Ok(())
}

fn rule<'a>(rs: &'a [NotificationRule], cat: &str) -> Option<&'a NotificationRule> {
    rs.iter().find(|r| r.category == cat)
}

pub struct Candidate {
    pub fingerprint: String,
    pub category: String,
    pub priority: i64,
    pub title: String,
    pub body: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub group_key: Option<String>,
    pub deliver_after: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Insert if new. Returns true when a new record was created.
pub fn enqueue(conn: &Connection, c: &Candidate) -> Result<bool> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO notifications(id, fingerprint, category, priority, title, body, entity_type, entity_id, group_key, state, deliver_after, expires_at, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'pending',?10,?11,?12)",
        params![
            new_id(), c.fingerprint, c.category, c.priority, text::truncate(&c.title, 120), text::truncate(&c.body, 400),
            c.entity_type, c.entity_id, c.group_key, ts(&c.deliver_after), c.expires_at.as_ref().map(ts), ts(&now())
        ],
    )?;
    Ok(n > 0)
}

fn fmt_rel(due: DateTime<Utc>, at: DateTime<Utc>, prefs: &Preferences) -> String {
    let tz = prefs.tz();
    let mins = (due - at).num_minutes();
    let local = due.with_timezone(&tz);
    if mins < 0 {
        let h = -mins / 60;
        if h < 24 {
            format!("overdue by {}", if h == 0 { format!("{} min", -mins) } else { format!("{h} h") })
        } else {
            format!("overdue since {}", local.format("%a %d %b"))
        }
    } else if mins < 120 {
        format!("due in {mins} min ({})", local.format("%H:%M"))
    } else if local.date_naive() == at.with_timezone(&tz).date_naive() {
        format!("due today at {}", local.format("%H:%M"))
    } else if local.date_naive() == at.with_timezone(&tz).date_naive() + Duration::days(1) {
        format!("due tomorrow at {}", local.format("%H:%M"))
    } else {
        format!("due {}", local.format("%a %d %b %H:%M"))
    }
}

/// Generate candidates from current state. Safe to call every minute.
pub fn generate(conn: &Connection, prefs: &Preferences, at: DateTime<Utc>) -> Result<usize> {
    let rs = rules(conn)?;
    let mut created = 0;
    let tz = prefs.tz();
    let today = at.with_timezone(&tz).date_naive();

    // 1. Deadlines and overdue work.
    let deadline = rule(&rs, "deadline").filter(|r| r.enabled);
    let overdue = rule(&rs, "overdue").filter(|r| r.enabled);
    let critical = rule(&rs, "critical").filter(|r| r.enabled);
    let mut st = conn.prepare(
        "SELECT id, title, due_at, priority, due_kind FROM tasks
         WHERE due_at IS NOT NULL AND needs_confirmation=0 AND my_role IN ('owner','collaborator')
           AND status IN ('ready','in_progress','blocked','draft_prepared','detected')",
    )?;
    let rows: Vec<(String, String, String, i64, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<rusqlite::Result<_>>()?;
    for (id, title, due, prio, kind) in rows {
        let Some(due) = parse_ts(&due) else { continue };
        let until = due - at;
        let kind_note = if kind == "inferred" { " (inferred deadline)" } else { "" };
        if until < Duration::zero() {
            if let Some(r) = overdue {
                if prio <= r.min_priority || prio <= 2 {
                    let rep = r.repeat_minutes.max(60);
                    let stage = (-until).num_minutes() / rep;
                    if stage <= 6 {
                        // Escalate: after the first reminder, overdue P1/P2 work goes up a level.
                        let p = if stage >= 1 { (prio - 1).max(1) } else { prio };
                        created += enqueue(conn, &Candidate {
                            fingerprint: format!("overdue:{id}:{}:{stage}", due.timestamp()),
                            category: "overdue".into(),
                            priority: p,
                            title: format!("Overdue: {title}"),
                            body: format!("{}{kind_note}. Open Kept to reschedule, mark done, or mark as waiting.", fmt_rel(due, at, prefs)),
                            entity_type: Some("task".into()),
                            entity_id: Some(id.clone()),
                            group_key: Some(format!("overdue:{today}")),
                            deliver_after: at,
                            expires_at: Some(at + Duration::minutes(rep)),
                        })? as usize;
                    }
                }
            }
            continue;
        }
        // Current stage only.
        if let Some(c) = critical {
            if prio <= c.min_priority.max(1) && until <= Duration::minutes(c.lead_minutes) {
                created += enqueue(conn, &Candidate {
                    fingerprint: format!("deadline:{id}:{}:critical", due.timestamp()),
                    category: "critical".into(),
                    priority: 1,
                    title: format!("Due soon: {title}"),
                    body: format!("{}{kind_note}.", fmt_rel(due, at, prefs)),
                    entity_type: Some("task".into()),
                    entity_id: Some(id.clone()),
                    group_key: None,
                    deliver_after: at,
                    expires_at: Some(due),
                })? as usize;
                continue;
            }
        }
        if let Some(r) = deadline {
            let lead = Duration::minutes(r.lead_minutes.max(15));
            let short = Duration::minutes(120);
            if prio <= r.min_priority && until <= lead {
                let stage = if until <= short && prio <= 2 { "2h" } else { "lead" };
                created += enqueue(conn, &Candidate {
                    fingerprint: format!("deadline:{id}:{}:{stage}", due.timestamp()),
                    category: "deadline".into(),
                    priority: prio,
                    title: if stage == "2h" { format!("Due soon: {title}") } else { format!("Coming up: {title}") },
                    body: format!("{}{kind_note}.", fmt_rel(due, at, prefs)),
                    entity_type: Some("task".into()),
                    entity_id: Some(id.clone()),
                    group_key: if stage == "lead" { Some(format!("due:{}", due.with_timezone(&tz).date_naive())) } else { None },
                    deliver_after: at,
                    expires_at: Some(due),
                })? as usize;
            }
        }
    }

    // 2. Meetings that need preparation.
    if let Some(r) = rule(&rs, "meeting_prep").filter(|r| r.enabled) {
        let lead = Duration::minutes(r.lead_minutes.max(15));
        for m in meetings::list_range(conn, at, at + lead)? {
            if m.prep_state != "needed" || m.status == "cancelled" || m.start_at <= at {
                continue;
            }
            let open: Vec<String> = {
                let mut st = conn.prepare("SELECT title FROM tasks WHERE meeting_id=?1 AND status NOT IN ('completed','cancelled','rejected') LIMIT 3")?;
                let v = st.query_map([&m.id], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                v
            };
            created += enqueue(conn, &Candidate {
                fingerprint: format!("meeting_prep:{}:{}", m.id, m.start_at.timestamp()),
                category: "meeting_prep".into(),
                priority: 2,
                title: format!("Prepare for \"{}\" at {}", m.title, m.start_at.with_timezone(&tz).format("%H:%M")),
                body: format!("Still open: {}", open.join("; ")),
                entity_type: Some("meeting".into()),
                entity_id: Some(m.id.clone()),
                group_key: None,
                deliver_after: at,
                expires_at: Some(m.start_at),
            })? as usize;
        }
        // After a meeting with linked work: prompt to capture follow-ups (in-app by default via priority 4).
        let mut st = conn.prepare("SELECT id, title FROM meetings WHERE end_at <= ?1 AND end_at > ?2 AND prep_state IN ('needed','ready') AND status <> 'cancelled'")?;
        let ended: Vec<(String, String)> = st.query_map(params![ts(&at), ts(&(at - Duration::hours(3)))], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        for (mid, title) in ended {
            created += enqueue(conn, &Candidate {
                fingerprint: format!("after_meeting:{mid}"),
                category: "followup".into(),
                priority: 4,
                title: format!("Any follow-ups from \"{title}\"?"),
                body: "Add action items you agreed to, so they don't get lost.".into(),
                entity_type: Some("meeting".into()),
                entity_id: Some(mid),
                group_key: None,
                deliver_after: at,
                expires_at: Some(at + Duration::days(2)),
            })? as usize;
        }
    }

    // 3. Review queue — consolidated, never urgent.
    if rule(&rs, "review").map(|r| r.enabled).unwrap_or(false) {
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM tasks WHERE needs_confirmation=1 AND status NOT IN ('completed','cancelled','rejected')", [], |r| r.get(0))?;
        let bucket = match n {
            0 => 0,
            1..=4 => 1,
            5..=9 => 5,
            10..=19 => 10,
            _ => 20,
        };
        if bucket > 0 {
            created += enqueue(conn, &Candidate {
                fingerprint: format!("review:{today}:{bucket}"),
                category: "review".into(),
                priority: 4,
                title: format!("{n} detected item{} to review", if n == 1 { "" } else { "s" }),
                body: "Kept found possible commitments it isn't sure about. Accept, edit, or reject them in Review.".into(),
                entity_type: None,
                entity_id: None,
                group_key: None,
                deliver_after: at,
                expires_at: Some(at + Duration::days(1)),
            })? as usize;
        }
    }

    // 4. Someone chased you / a deadline moved (from the audit log, last 2 days).
    if let Some(r) = rule(&rs, "followup").filter(|r| r.enabled) {
        let mut st = conn.prepare(
            "SELECT a.id, a.entity_id, a.reason, t.title, a.action, a.from_value, a.to_value FROM audit_events a JOIN tasks t ON t.id=a.entity_id
             WHERE a.entity_type='task' AND a.created_at > ?1 AND a.actor='agent' AND a.action IN ('nudged','due')
               AND t.status NOT IN ('completed','cancelled','rejected')",
        )?;
        let rows: Vec<(String, String, Option<String>, String, String, Option<String>, Option<String>)> =
            st.query_map([ts(&(at - Duration::days(2)))], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?.collect::<rusqlite::Result<_>>()?;
        for (aid, tid, reason, title, action, from, to) in rows {
            if action == "nudged" {
                created += enqueue(conn, &Candidate {
                    fingerprint: format!("nudge:{aid}"),
                    category: "followup".into(),
                    priority: 2.min(r.min_priority),
                    title: format!("Follow-up: {title}"),
                    body: reason.unwrap_or_else(|| "Someone followed up on this.".into()),
                    entity_type: Some("task".into()),
                    entity_id: Some(tid),
                    group_key: None,
                    deliver_after: at,
                    expires_at: Some(at + Duration::days(2)),
                })? as usize;
            } else {
                // Deadline change: notify only if it got earlier or is now close.
                let (Some(f), Some(t)) = (from.as_deref().and_then(parse_ts), to.as_deref().and_then(parse_ts)) else { continue };
                if t < f || (t - at) < Duration::days(3) {
                    created += enqueue(conn, &Candidate {
                        fingerprint: format!("due_changed:{aid}"),
                        category: "deadline".into(),
                        priority: 2,
                        title: format!("Deadline moved: {title}"),
                        body: format!("Now {} (was {}). {}", t.with_timezone(&tz).format("%a %d %b %H:%M"), f.with_timezone(&tz).format("%a %d %b %H:%M"), reason.unwrap_or_default()),
                        entity_type: Some("task".into()),
                        entity_id: Some(tid),
                        group_key: None,
                        deliver_after: at,
                        expires_at: Some(at + Duration::days(1)),
                    })? as usize;
                }
            }
        }
    }

    // 5. Integrations that need attention (once per account/state/day).
    if rule(&rs, "sync").map(|r| r.enabled).unwrap_or(false) {
        let mut st = conn.prepare("SELECT id, display_name, status, status_detail FROM accounts WHERE status IN ('attention','error')")?;
        let rows: Vec<(String, String, String, Option<String>)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
        for (id, name, stt, detail) in rows {
            created += enqueue(conn, &Candidate {
                fingerprint: format!("sync:{id}:{stt}:{today}"),
                category: "sync".into(),
                priority: 3,
                title: format!("{name} needs attention"),
                body: detail.unwrap_or_else(|| "Synchronisation is failing. Open Integrations to reconnect.".into()),
                entity_type: Some("account".into()),
                entity_id: Some(id),
                group_key: None,
                deliver_after: at,
                expires_at: Some(at + Duration::days(1)),
            })? as usize;
        }
    }

    // 6. Daily digest / end-of-day review / overload warning (once per day, working days).
    let local = at.with_timezone(&tz);
    let working_day = prefs.working_days.contains(&local.weekday().number_from_monday()) && !prefs.holidays.contains(&today);
    if working_day && rule(&rs, "digest").map(|r| r.enabled).unwrap_or(false) {
        let due_time = |s: &str| parse_hm(s).map(|t| local.time() >= t && local.time() < t + Duration::hours(3)).unwrap_or(false);
        if prefs.digest_enabled && due_time(&prefs.digest_time) {
            let (title, body) = planner::digest_text(conn, prefs, false)?;
            created += enqueue(conn, &Candidate {
                fingerprint: format!("digest:{today}"),
                category: "digest".into(),
                priority: 3,
                title,
                body,
                entity_type: None,
                entity_id: None,
                group_key: None,
                deliver_after: at,
                expires_at: Some(at + Duration::hours(4)),
            })? as usize;
        }
        if prefs.eod_review_enabled && due_time(&prefs.eod_review_time) {
            let (title, body) = planner::digest_text(conn, prefs, true)?;
            created += enqueue(conn, &Candidate {
                fingerprint: format!("eod:{today}"),
                category: "digest".into(),
                priority: 3,
                title,
                body,
                entity_type: None,
                entity_id: None,
                group_key: None,
                deliver_after: at,
                expires_at: Some(at + Duration::hours(4)),
            })? as usize;
        }
    }
    if working_day && rule(&rs, "deadline").map(|r| r.enabled).unwrap_or(false) {
        let in_hours = parse_hm(&prefs.work_start).map(|s| local.time() >= s).unwrap_or(true) && parse_hm(&prefs.work_end).map(|e| local.time() < e).unwrap_or(true);
        if in_hours {
            let plan = planner::day_plan(conn, prefs, at)?;
            if plan.overloaded && plan.required_minutes > 0 {
                created += enqueue(conn, &Candidate {
                    fingerprint: format!("overload:{today}"),
                    category: "deadline".into(),
                    priority: 2,
                    title: "Today is over capacity".into(),
                    body: format!("{} Consider renegotiating a deadline or moving lower-priority work.", plan.warnings.first().cloned().unwrap_or_default()),
                    entity_type: None,
                    entity_id: None,
                    group_key: None,
                    deliver_after: at,
                    expires_at: Some(at + Duration::hours(8)),
                })? as usize;
            }
        }
    }
    Ok(created)
}

pub fn in_quiet_hours(prefs: &Preferences, at: DateTime<Utc>) -> bool {
    if !prefs.quiet_hours_enabled {
        return false;
    }
    let (Some(s), Some(e)) = (parse_hm(&prefs.quiet_start), parse_hm(&prefs.quiet_end)) else { return false };
    let t: NaiveTime = at.with_timezone(&prefs.tz()).time();
    if s <= e {
        t >= s && t < e
    } else {
        t >= s || t < e
    }
}

/// Decide which pending notifications become desktop toasts now. Marks them delivered.
pub fn due_toasts(conn: &Connection, prefs: &Preferences, at: DateTime<Utc>) -> Result<Vec<Toast>> {
    let rs = rules(conn)?;
    let nts = ts(&at);
    // Expire stale ones first (e.g. after waking from sleep) so we don't flood.
    conn.execute("UPDATE notifications SET state='expired' WHERE state IN ('pending','held','snoozed') AND expires_at IS NOT NULL AND expires_at < ?1", [&nts])?;
    conn.execute("UPDATE notifications SET state='pending', snoozed_until=NULL WHERE state='snoozed' AND snoozed_until <= ?1", [&nts])?;
    let paused = prefs.monitoring_paused || prefs.notifications_paused_until.map(|u| u > at).unwrap_or(false);
    let quiet = in_quiet_hours(prefs, at);

    let mut st = conn.prepare("SELECT id, category, priority, title, body, group_key FROM notifications WHERE state IN ('pending','held') AND deliver_after <= ?1 ORDER BY priority, created_at")?;
    let rows: Vec<(String, String, i64, String, String, Option<String>)> = st.query_map([&nts], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect::<rusqlite::Result<_>>()?;
    let sent_last_hour: i64 = conn.query_row(
        "SELECT COUNT(*) FROM notifications WHERE state IN ('delivered','failed') AND delivered_at > ?1",
        [ts(&(at - Duration::hours(1)))],
        |r| r.get(0),
    )?;
    let mut budget = prefs.max_notifications_per_hour as i64 - sent_last_hour;
    let mut toasts: Vec<Toast> = vec![];
    let mut grouped: std::collections::BTreeMap<String, Vec<(String, String, i64)>> = Default::default();

    for (id, cat, prio, title, body, group) in rows {
        let r = rule(&rs, &cat);
        let desktop = r.map(|r| r.enabled && r.desktop && prio <= r.min_priority).unwrap_or(true);
        if !desktop {
            conn.execute("UPDATE notifications SET state='in_app', delivered_at=?1 WHERE id=?2", params![nts, id])?;
            continue;
        }
        let bypass = r.map(|r| r.bypass_quiet).unwrap_or(false) && prio == 1;
        if paused || (quiet && !bypass) {
            conn.execute("UPDATE notifications SET state='held' WHERE id=?1 AND state='pending'", [&id])?;
            continue;
        }
        if budget <= 0 && prio > 1 {
            continue; // stays pending; delivered next hour (or consolidated)
        }
        match group {
            Some(g) => grouped.entry(g).or_default().push((id, title, prio)),
            None => {
                toasts.push(Toast { notification_id: id.clone(), title, body, priority: prio });
                conn.execute("UPDATE notifications SET state='delivered', delivered_at=?1 WHERE id=?2", params![nts, id])?;
                budget -= 1;
            }
        }
    }
    for (g, items) in grouped {
        if budget <= 0 && items.iter().all(|i| i.2 > 1) {
            continue; // rate limit reached: stays pending for the next window
        }
        if items.len() == 1 {
            let (id, title, prio) = &items[0];
            let body: String = conn.query_row("SELECT body FROM notifications WHERE id=?1", [id], |r| r.get(0))?;
            toasts.push(Toast { notification_id: id.clone(), title: title.clone(), body, priority: *prio });
        } else {
            let label = if g.starts_with("overdue") { "overdue tasks" } else { "tasks coming up" };
            let names: Vec<String> = items.iter().take(3).map(|(_, t, _)| t.trim_start_matches("Coming up: ").trim_start_matches("Overdue: ").to_string()).collect();
            let more = if items.len() > 3 { format!(" and {} more", items.len() - 3) } else { String::new() };
            toasts.push(Toast {
                notification_id: items[0].0.clone(),
                title: format!("{} {label}", items.len()),
                body: format!("{}{more}", names.join(" · ")),
                priority: items.iter().map(|i| i.2).min().unwrap_or(3),
            });
        }
        for (id, _, _) in &items {
            conn.execute("UPDATE notifications SET state='delivered', delivered_at=?1 WHERE id=?2", params![nts, id])?;
        }
        budget -= 1;
    }
    Ok(toasts)
}

pub fn mark_failed(conn: &Connection, id: &str, err: &str) -> Result<()> {
    conn.execute("UPDATE notifications SET state='failed', delivery_error=?1 WHERE id=?2", params![text::truncate(err, 200), id])?;
    Ok(())
}

pub fn snooze(conn: &Connection, id: &str, minutes: i64) -> Result<()> {
    let until = now() + Duration::minutes(minutes.clamp(5, 60 * 24 * 7));
    conn.execute("UPDATE notifications SET state='snoozed', snoozed_until=?1, read_at=COALESCE(read_at, ?2) WHERE id=?3", params![ts(&until), ts(&now()), id])?;
    // Make sure the snoozed alert outlives its original expiry.
    conn.execute("UPDATE notifications SET expires_at=NULL WHERE id=?1 AND expires_at < ?2", params![id, ts(&until)])?;
    Ok(())
}

pub fn dismiss(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("UPDATE notifications SET state='dismissed', read_at=COALESCE(read_at, ?1) WHERE id=?2", params![ts(&now()), id])?;
    Ok(())
}

pub fn mark_read(conn: &Connection, id: Option<&str>) -> Result<()> {
    match id {
        Some(i) => conn.execute("UPDATE notifications SET read_at=COALESCE(read_at, ?1) WHERE id=?2", params![ts(&now()), i])?,
        None => conn.execute("UPDATE notifications SET read_at=?1 WHERE read_at IS NULL", [ts(&now())])?,
    };
    Ok(())
}

pub fn list(conn: &Connection, scope: &str, limit: i64) -> Result<Vec<Notification>> {
    let where_ = match scope {
        "pending" => "state IN ('pending','held')",
        "snoozed" => "state='snoozed'",
        "unread" => "read_at IS NULL AND state IN ('delivered','in_app','failed')",
        _ => "state <> 'pending'",
    };
    let mut st = conn.prepare(&format!(
        "SELECT id, fingerprint, category, priority, title, body, entity_type, entity_id, group_key, state, deliver_after, delivered_at, delivery_error, read_at, snoozed_until, expires_at, created_at
         FROM notifications WHERE {where_} ORDER BY created_at DESC LIMIT ?1"
    ))?;
    let v = st
        .query_map([limit], |r| {
            Ok(Notification {
                id: r.get(0)?,
                fingerprint: r.get(1)?,
                category: r.get(2)?,
                priority: r.get(3)?,
                title: r.get(4)?,
                body: r.get(5)?,
                entity_type: r.get(6)?,
                entity_id: r.get(7)?,
                group_key: r.get(8)?,
                state: r.get(9)?,
                deliver_after: opt_ts(r.get(10)?).unwrap_or_else(Utc::now),
                delivered_at: opt_ts(r.get(11)?),
                delivery_error: r.get(12)?,
                read_at: opt_ts(r.get(13)?),
                snoozed_until: opt_ts(r.get(14)?),
                expires_at: opt_ts(r.get(15)?),
                created_at: opt_ts(r.get(16)?).unwrap_or_else(Utc::now),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(v)
}

pub fn unread_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM notifications WHERE read_at IS NULL AND state IN ('delivered','in_app','failed')", [], |r| r.get(0))?)
}

/// Record that a toast was clicked (acted upon).
pub fn acted(conn: &Connection, id: &str) -> Result<Option<(String, String)>> {
    conn.execute("UPDATE notifications SET acted_at=?1, read_at=COALESCE(read_at, ?1) WHERE id=?2", params![ts(&now()), id])?;
    Ok(conn
        .query_row("SELECT entity_type, entity_id FROM notifications WHERE id=?1 AND entity_type IS NOT NULL", [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?)
}

pub fn _touch(conn: &Connection) -> Result<()> {
    prefs::set_state(conn, "notify_last", &ts(&now()))
}
