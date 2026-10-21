//! Deterministic planning: priority scoring, effort estimates, daily plan, workload,
//! meeting conflicts, and the home-dashboard summary.

use crate::db::{now, opt_ts, ts};
use crate::extract::rules::workday;
use crate::models::{Meeting, Task};
use crate::prefs::Preferences;
use crate::tasks::{self, TaskFilter};
use crate::{dates, meetings};
use anyhow::Result;
use chrono::{DateTime, Duration, Timelike, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;

pub fn estimate_effort(task_type: &str, title: &str) -> i64 {
    let t = title.to_lowercase();
    let by_word = [
        ("report", 120), ("analysis", 120), ("proposal", 120), ("budget", 90), ("forecast", 90), ("slides", 90), ("deck", 90),
        ("presentation", 90), ("spec", 90), ("plan", 60), ("draft", 60), ("document", 60), ("contract", 45), ("summary", 30),
        ("agenda", 20), ("notes", 20), ("invoice", 15),
    ];
    if let Some((_, m)) = by_word.iter().find(|(w, _)| t.contains(w)) {
        if matches!(task_type, "deliverable" | "action" | "prep") {
            return *m;
        }
    }
    match task_type {
        "reply" => 10,
        "followup" => 10,
        "approval" => 15,
        "review" => 30,
        "prep" => 30,
        "deliverable" => 60,
        _ => 30,
    }
}

/// Score → priority (1 critical .. 4 low) with a human-readable reason.
pub fn score(t: &Task, prefs: &Preferences, meeting_start: Option<DateTime<Utc>>, at: DateTime<Utc>) -> (i64, String) {
    let mut s = 0i32;
    let mut why: Vec<String> = vec![];
    if let Some(d) = t.due_at {
        let h = (d - at).num_minutes() as f64 / 60.0;
        if h < 0.0 {
            s += 4;
            why.push("overdue".into());
        } else if h <= 24.0 {
            s += 3;
            why.push("due within 24h".into());
        } else if h <= 72.0 {
            s += 2;
            why.push("due within 3 days".into());
        } else if h <= 168.0 {
            s += 1;
            why.push("due this week".into());
        }
    }
    if matches!(t.task_type.as_str(), "approval") {
        s += 1;
        why.push("others are blocked on your decision".into());
    }
    if t.task_type == "deliverable" {
        s += 1;
        why.push("promised deliverable".into());
    }
    if let Some(m) = meeting_start {
        if m > at && (m - at) <= Duration::hours(48) {
            s += 1;
            why.push("needed for an upcoming meeting".into());
        }
    }
    if let Some(cp) = &t.counterpart_email {
        if prefs.vip_emails.iter().any(|v| v.eq_ignore_ascii_case(cp)) {
            s += 1;
            why.push("for a VIP contact".into());
        }
    }
    if t.confidence < 0.6 {
        s -= 1;
        why.push("low confidence".into());
    }
    let p = match s {
        x if x >= 5 => 1,
        x if x >= 3 => 2,
        x if x >= 1 => 3,
        _ => 4,
    };
    let reason = if why.is_empty() { "no deadline pressure".to_string() } else { why.join(" · ") };
    (p, reason)
}

fn meeting_start(conn: &Connection, id: &Option<String>) -> Option<DateTime<Utc>> {
    id.as_ref().and_then(|m| conn.query_row("SELECT start_at FROM meetings WHERE id=?1", [m], |r| r.get::<_, String>(0)).ok()).and_then(|s| crate::db::parse_ts(&s))
}

pub fn reprioritize_one(conn: &Connection, prefs: &Preferences, id: &str) -> Result<()> {
    let t = tasks::get(conn, id)?;
    let locked: bool = conn.query_row("SELECT priority_locked FROM tasks WHERE id=?1", [id], |r| r.get(0))?;
    if locked {
        return Ok(());
    }
    let (p, why) = score(&t, prefs, meeting_start(conn, &t.meeting_id), now());
    conn.execute("UPDATE tasks SET priority=?1, priority_reason=?2 WHERE id=?3", params![p, why, id])?;
    Ok(())
}

/// Recompute priorities for open tasks (deadlines approach as time passes).
pub fn reprioritize_all(conn: &Connection, prefs: &Preferences) -> Result<usize> {
    let open = tasks::list(conn, &TaskFilter::default())?;
    let mut changed = 0;
    let at = now();
    for t in open {
        let locked: bool = conn.query_row("SELECT priority_locked FROM tasks WHERE id=?1", [&t.id], |r| r.get(0))?;
        if locked {
            continue;
        }
        let (p, why) = score(&t, prefs, meeting_start(conn, &t.meeting_id), at);
        if p != t.priority || Some(&why) != t.priority_reason.as_ref() {
            conn.execute("UPDATE tasks SET priority=?1, priority_reason=?2 WHERE id=?3", params![p, why, t.id])?;
            changed += 1;
        }
    }
    Ok(changed)
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanBlock {
    pub task_id: Option<String>,
    pub meeting_id: Option<String>,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub kind: String, // task | meeting
}

#[derive(Debug, Clone, Serialize)]
pub struct DayPlan {
    pub date: String,
    pub available_minutes: i64,
    pub meeting_minutes: i64,
    pub planned_minutes: i64,
    pub required_minutes: i64,
    pub overloaded: bool,
    pub blocks: Vec<PlanBlock>,
    pub unscheduled: Vec<Task>,
    pub warnings: Vec<String>,
    pub working_day: bool,
}

fn ordering_key(t: &Task, style: &str) -> (i64, i64, i64) {
    let due = t.due_at.map(|d| d.timestamp()).unwrap_or(i64::MAX);
    let eff = t.effort_minutes.unwrap_or(30);
    match style {
        "deadline_first" => (due, t.priority, eff),
        "quick_wins" => (t.priority, eff, due),
        _ => (t.priority, due, eff),
    }
}

/// An achievable plan for the rest of today: tasks fitted into gaps between meetings.
pub fn day_plan(conn: &Connection, prefs: &Preferences, at: DateTime<Utc>) -> Result<DayPlan> {
    let tz = prefs.tz();
    let wd = workday(prefs);
    let local = at.with_timezone(&tz);
    let date = local.date_naive();
    let day_start = dates::local_to_utc(tz, date, wd.start);
    let day_end = dates::local_to_utc(tz, date, wd.end);
    let working_day = prefs.working_days.contains(&chrono::Datelike::weekday(&local).number_from_monday()) && !prefs.holidays.contains(&date);
    let from = at.max(day_start);
    let mut warnings = vec![];

    let ms = meetings::list_range(conn, day_start - Duration::hours(1), day_end + Duration::hours(1))?;
    let ms: Vec<Meeting> = ms.into_iter().filter(|m| m.status != "cancelled" && !m.all_day && m.my_response.as_deref() != Some("declined")).collect();
    let mut busy: Vec<(DateTime<Utc>, DateTime<Utc>)> = ms.iter().map(|m| (m.start_at.max(day_start), m.end_at.min(day_end))).filter(|(a, b)| b > a).collect();
    busy.sort();
    for w in ms.windows(2) {
        if w[1].start_at < w[0].end_at {
            warnings.push(format!("\"{}\" overlaps \"{}\"", w[0].title, w[1].title));
        }
    }
    let meeting_minutes: i64 = busy.iter().filter(|(_, b)| *b > from).map(|(a, b)| (*b - (*a).max(from)).num_minutes().max(0)).sum();
    let available = if working_day && day_end > from { (day_end - from).num_minutes() - meeting_minutes } else { 0 };

    let mut candidates: Vec<Task> = tasks::list(conn, &TaskFilter { statuses: vec!["ready".into(), "in_progress".into(), "blocked".into()], ..Default::default() })?
        .into_iter()
        .filter(|t| t.status != "blocked" && t.my_role == "owner")
        .collect();
    candidates.sort_by_key(|t| ordering_key(t, &prefs.planning_style));
    // What *must* happen today: due today or overdue.
    let required: i64 = candidates.iter().filter(|t| t.due_at.map(|d| d <= day_end + Duration::hours(1)).unwrap_or(false)).map(|t| t.effort_minutes.unwrap_or(30)).sum();

    // Free slots.
    let mut slots: Vec<(DateTime<Utc>, DateTime<Utc>)> = vec![];
    if working_day && day_end > from {
        let mut cur = from;
        for (a, b) in &busy {
            if *a > cur {
                slots.push((cur, *a));
            }
            cur = cur.max(*b);
        }
        if day_end > cur {
            slots.push((cur, day_end));
        }
    }
    // Round slot starts to 5 minutes.
    let slots: Vec<_> = slots
        .into_iter()
        .map(|(a, b)| {
            let extra = (5 - (a.minute() % 5)) % 5;
            (a + Duration::minutes(extra as i64) - Duration::seconds(a.second() as i64), b)
        })
        .filter(|(a, b)| (*b - *a).num_minutes() >= 10)
        .collect();

    let mut blocks: Vec<PlanBlock> = ms
        .iter()
        .filter(|m| m.end_at > from)
        .map(|m| PlanBlock { task_id: None, meeting_id: Some(m.id.clone()), title: m.title.clone(), start: m.start_at, end: m.end_at, kind: "meeting".into() })
        .collect();
    let mut unscheduled = vec![];
    let mut slot_state: Vec<(DateTime<Utc>, DateTime<Utc>)> = slots.clone();
    let mut planned = 0;
    for t in candidates.into_iter() {
        let need = t.effort_minutes.unwrap_or(30).clamp(5, 240);
        let deadline = t.due_at;
        let mut placed = false;
        for s in slot_state.iter_mut() {
            let len = (s.1 - s.0).num_minutes();
            if len >= need {
                let end = s.0 + Duration::minutes(need);
                if deadline.map(|d| end > d && d > from).unwrap_or(false) {
                    continue; // would finish after its deadline in this slot
                }
                blocks.push(PlanBlock { task_id: Some(t.id.clone()), meeting_id: t.meeting_id.clone(), title: t.title.clone(), start: s.0, end, kind: "task".into() });
                s.0 = end;
                planned += need;
                placed = true;
                break;
            }
        }
        if !placed {
            if t.due_at.map(|d| d <= day_end).unwrap_or(false) {
                warnings.push(format!("No room today for \"{}\" (due {})", t.title, t.due_at.unwrap().with_timezone(&tz).format("%a %H:%M")));
            }
            unscheduled.push(t);
        }
    }
    blocks.sort_by_key(|b| b.start);
    let overloaded = working_day && required > available.max(0);
    if overloaded {
        warnings.insert(0, format!("Work due today needs about {} h but only {} h are free.", (required as f64 / 60.0 * 10.0).round() / 10.0, (available.max(0) as f64 / 60.0 * 10.0).round() / 10.0));
    }
    unscheduled.truncate(30);
    Ok(DayPlan {
        date: date.to_string(),
        available_minutes: available.max(0),
        meeting_minutes,
        planned_minutes: planned,
        required_minutes: required,
        overloaded,
        blocks,
        unscheduled,
        warnings,
        working_day,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivityItem {
    pub at: DateTime<Utc>,
    pub text: String,
    pub task_id: Option<String>,
    pub actor: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Dashboard {
    pub greeting: String,
    pub local_time: String,
    pub briefing: Vec<String>,
    pub next_up: Option<Task>,
    pub priorities: Vec<Task>,
    pub due_today: Vec<Task>,
    pub overdue: Vec<Task>,
    pub review: Vec<Task>,
    pub review_count: i64,
    pub blocked: Vec<Task>,
    pub waiting: Vec<Task>,
    pub meetings: Vec<Meeting>,
    pub meetings_needing_prep: i64,
    pub followups_count: i64,
    pub recent: Vec<ActivityItem>,
    pub plan: DayPlan,
    pub unread_notifications: i64,
    pub attention_accounts: Vec<String>,
    pub monitoring_paused: bool,
}

pub fn dashboard(conn: &Connection, prefs: &Preferences) -> Result<Dashboard> {
    let at = now();
    let tz = prefs.tz();
    let local = at.with_timezone(&tz);
    let hour = local.hour();
    let part = if hour < 12 { "morning" } else if hour < 17 { "afternoon" } else { "evening" };
    let name = prefs.user_name.split_whitespace().next().unwrap_or("").to_string();
    let greeting = if name.is_empty() { format!("Good {part}") } else { format!("Good {part}, {name}") };
    let wd = workday(prefs);
    let end_today = dates::local_to_utc(tz, local.date_naive(), chrono::NaiveTime::from_hms_opt(23, 59, 0).unwrap());
    let _ = wd;

    let open = tasks::list(conn, &TaskFilter { sort: "priority".into(), ..Default::default() })?;
    let active: Vec<Task> = open.iter().filter(|t| !t.needs_confirmation).cloned().collect();
    let overdue: Vec<Task> = active.iter().filter(|t| t.due_at.map(|d| d < at).unwrap_or(false) && !matches!(t.status.as_str(), "waiting" | "awaiting_approval")).cloned().collect();
    let due_today: Vec<Task> = active.iter().filter(|t| t.due_at.map(|d| d >= at && d <= end_today).unwrap_or(false)).cloned().collect();
    let blocked: Vec<Task> = active.iter().filter(|t| t.status == "blocked").cloned().collect();
    let waiting: Vec<Task> = active.iter().filter(|t| matches!(t.status.as_str(), "waiting" | "awaiting_approval" | "sent")).cloned().collect();
    let mut priorities: Vec<Task> = active.iter().filter(|t| matches!(t.status.as_str(), "ready" | "in_progress" | "draft_prepared") && t.my_role == "owner").cloned().collect();
    priorities.sort_by_key(|t| (t.priority, t.due_at.map(|d| d.timestamp()).unwrap_or(i64::MAX)));
    let next_up = priorities.first().cloned();
    priorities.truncate(6);
    let review: Vec<Task> = open.iter().filter(|t| t.needs_confirmation).take(5).cloned().collect();
    let review_count = open.iter().filter(|t| t.needs_confirmation).count() as i64;

    let ms = meetings::list_range(conn, at - Duration::minutes(30), at + Duration::days(7))?;
    let ms: Vec<Meeting> = ms.into_iter().filter(|m| m.status != "cancelled").collect();
    let needing_prep = ms.iter().filter(|m| m.prep_state == "needed").count() as i64;
    let followups_count: i64 = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM commitments WHERE status='open' AND direction='theirs')
              + (SELECT COUNT(*) FROM tasks WHERE status IN ('waiting','awaiting_approval','sent') )
              + (SELECT COUNT(*) FROM tasks WHERE task_type IN ('reply','followup') AND status IN ('ready','in_progress'))",
        [],
        |r| r.get(0),
    )?;
    let mut st = conn.prepare(
        "SELECT a.created_at, a.action, a.to_value, a.actor, a.reason, a.entity_id, t.title FROM audit_events a
         LEFT JOIN tasks t ON t.id = a.entity_id WHERE a.entity_type='task' AND a.action IN ('created','status','merged','accepted','due','nudged')
         ORDER BY a.created_at DESC LIMIT 12",
    )?;
    let recent: Vec<ActivityItem> = st
        .query_map([], |r| {
            let action: String = r.get(1)?;
            let to: Option<String> = r.get(2)?;
            let actor: String = r.get(3)?;
            let reason: Option<String> = r.get(4)?;
            let title: Option<String> = r.get(6)?;
            let title = title.unwrap_or_else(|| "(deleted task)".into());
            let text = match action.as_str() {
                "created" => format!("{} \"{title}\"", if actor == "user" { "You added" } else { "Detected" }),
                "status" => format!("\"{title}\" → {}", to.unwrap_or_default().replace('_', " ")),
                "merged" => format!("Merged a duplicate into \"{title}\""),
                "accepted" => format!("You accepted \"{title}\""),
                "due" => format!("Deadline changed for \"{title}\""),
                "nudged" => format!("{} on \"{title}\"", reason.unwrap_or_else(|| "Follow-up".into())),
                _ => title,
            };
            Ok(ActivityItem { at: opt_ts(r.get(0)?).unwrap_or_else(Utc::now), text, task_id: r.get(5)?, actor })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let unread: i64 = conn.query_row("SELECT COUNT(*) FROM notifications WHERE read_at IS NULL AND state IN ('delivered','in_app','failed')", [], |r| r.get(0))?;
    let mut st = conn.prepare("SELECT display_name FROM accounts WHERE status IN ('attention','error')")?;
    let attention: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let plan = day_plan(conn, prefs, at)?;

    // Concise deterministic briefing.
    let mut b = vec![];
    if !overdue.is_empty() {
        b.push(format!("{} overdue item{} — start with \"{}\".", overdue.len(), if overdue.len() == 1 { "" } else { "s" }, overdue[0].title));
    }
    if !due_today.is_empty() {
        b.push(format!("{} due today.", plural(due_today.len(), "task")));
    }
    let today_meetings: Vec<&Meeting> = ms.iter().filter(|m| m.start_at <= end_today).collect();
    if !today_meetings.is_empty() {
        let first = today_meetings.iter().find(|m| m.start_at > at);
        b.push(match first {
            Some(m) => format!("{} today; next is \"{}\" at {}.", plural(today_meetings.len(), "meeting"), m.title, m.start_at.with_timezone(&tz).format("%H:%M")),
            None => format!("{} today, all started or finished.", plural(today_meetings.len(), "meeting")),
        });
    }
    if needing_prep > 0 {
        b.push(format!("{} in the next week need{} preparation.", plural(needing_prep as usize, "meeting"), if needing_prep == 1 { "s" } else { "" }));
    }
    if review_count > 0 {
        b.push(format!("{} waiting for your review.", plural(review_count as usize, "detected item")));
    }
    if plan.overloaded {
        b.push(plan.warnings.first().cloned().unwrap_or_default());
    }
    if !waiting.is_empty() {
        b.push(format!("Waiting on others for {}.", plural(waiting.len(), "item")));
    }
    if b.is_empty() {
        b.push("Nothing urgent. A good moment for deep work.".into());
    }

    Ok(Dashboard {
        greeting,
        local_time: local.format("%A, %d %B · %H:%M").to_string(),
        briefing: b,
        next_up,
        priorities,
        due_today,
        overdue,
        review,
        review_count,
        blocked,
        waiting,
        meetings: ms.into_iter().take(6).collect(),
        meetings_needing_prep: needing_prep,
        followups_count,
        recent,
        plan,
        unread_notifications: unread,
        attention_accounts: attention,
        monitoring_paused: prefs.monitoring_paused,
    })
}

fn plural(n: usize, w: &str) -> String {
    format!("{n} {w}{}", if n == 1 { "" } else { "s" })
}

/// Text for the daily digest / end-of-day review notifications.
pub fn digest_text(conn: &Connection, prefs: &Preferences, eod: bool) -> Result<(String, String)> {
    let d = dashboard(conn, prefs)?;
    if eod {
        let done_today: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE status='completed' AND completed_at >= ?1",
            [ts(&(now() - Duration::hours(12)))],
            |r| r.get(0),
        )?;
        let open_due = d.due_today.len() + d.overdue.len();
        Ok((
            "End-of-day review".into(),
            format!("{done_today} completed today. {open_due} still due or overdue. {} awaiting review.", d.review_count),
        ))
    } else {
        Ok(("Your day with Kept".into(), d.briefing.join(" ")))
    }
}
