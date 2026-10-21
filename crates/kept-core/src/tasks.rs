//! Task repository: CRUD, lifecycle, deadlines, evidence, duplicates, merge and undo.
//! All mutations are audited.

use crate::db::{audit, new_id, now, opt_ts, parse_ts, ts};
use crate::models::{status, AuditEvent, Deadline, Person, SourceRef, Task, TaskDetail};
use crate::text;
use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

const SELECT: &str = "SELECT t.id, t.title, t.description, t.task_type, t.my_role, t.owner_name, t.status, t.priority,
    t.priority_reason, t.confidence, t.origin, t.needs_confirmation, t.due_at, t.due_kind, t.due_phrase, t.committed_at,
    t.meeting_id, m.title, p.name, t.expected_deliverable, t.completion_criteria, t.parent_id, t.recurrence,
    t.effort_minutes, t.people_json, t.counterpart_email, t.possible_duplicate_of, t.conflict_note, t.review_reason,
    t.completion_evidence, t.last_update_at, t.completed_at, t.created_at, t.updated_at,
    (SELECT COUNT(*) FROM task_sources s WHERE s.task_id = t.id),
    (SELECT COUNT(*) FROM tasks c WHERE c.parent_id = t.id),
    (SELECT COUNT(*) FROM tasks c WHERE c.parent_id = t.id AND c.status IN ('completed','cancelled'))
  FROM tasks t LEFT JOIN meetings m ON m.id = t.meeting_id LEFT JOIN projects p ON p.id = t.project_id";

fn row_to_task(r: &Row) -> rusqlite::Result<Task> {
    let people: String = r.get(24)?;
    Ok(Task {
        id: r.get(0)?,
        title: r.get(1)?,
        description: r.get(2)?,
        task_type: r.get(3)?,
        my_role: r.get(4)?,
        owner_name: r.get(5)?,
        status: r.get(6)?,
        priority: r.get(7)?,
        priority_reason: r.get(8)?,
        confidence: r.get(9)?,
        origin: r.get(10)?,
        needs_confirmation: r.get::<_, i64>(11)? != 0,
        due_at: opt_ts(r.get(12)?),
        due_kind: r.get(13)?,
        due_phrase: r.get(14)?,
        committed_at: opt_ts(r.get(15)?),
        meeting_id: r.get(16)?,
        meeting_title: r.get(17)?,
        project: r.get(18)?,
        expected_deliverable: r.get(19)?,
        completion_criteria: r.get(20)?,
        parent_id: r.get(21)?,
        recurrence: r.get(22)?,
        effort_minutes: r.get(23)?,
        people: serde_json::from_str(&people).unwrap_or_default(),
        counterpart_email: r.get(25)?,
        possible_duplicate_of: r.get(26)?,
        conflict_note: r.get(27)?,
        review_reason: r.get(28)?,
        completion_evidence: r.get(29)?,
        last_update_at: opt_ts(r.get(30)?),
        completed_at: opt_ts(r.get(31)?),
        created_at: opt_ts(r.get(32)?).unwrap_or_else(Utc::now),
        updated_at: opt_ts(r.get(33)?).unwrap_or_else(Utc::now),
        source_count: r.get(34)?,
        primary_source: None,
        subtask_count: r.get(35)?,
        subtasks_done: r.get(36)?,
    })
}

fn source_row(r: &Row) -> rusqlite::Result<SourceRef> {
    Ok(SourceRef {
        id: r.get(0)?,
        source_item_id: r.get(1)?,
        meeting_id: r.get(2)?,
        relation: r.get(3)?,
        evidence_quote: r.get(4)?,
        provider: r.get(5)?,
        url: r.get(6)?,
        label: r.get(7)?,
        added_at: opt_ts(r.get(8)?).unwrap_or_else(Utc::now),
    })
}

fn primary_source(conn: &Connection, task_id: &str) -> Option<SourceRef> {
    conn.query_row(
        "SELECT id, source_item_id, meeting_id, relation, evidence_quote, provider, url, source_label, added_at
         FROM task_sources WHERE task_id=?1 ORDER BY CASE relation WHEN 'origin' THEN 0 ELSE 1 END, added_at LIMIT 1",
        [task_id],
        source_row,
    )
    .optional()
    .ok()
    .flatten()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskFilter {
    pub statuses: Vec<String>,
    pub include_closed: bool,
    pub review_only: bool,
    pub search: String,
    pub meeting_id: Option<String>,
    pub parent_id: Option<String>,
    pub top_level_only: bool,
    pub due_before: Option<DateTime<Utc>>,
    pub task_type: Option<String>,
    /// due | priority | created | updated
    pub sort: String,
    pub limit: Option<i64>,
}

pub fn list(conn: &Connection, f: &TaskFilter) -> Result<Vec<Task>> {
    let mut sql = format!("{SELECT} WHERE 1=1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![];
    if f.review_only {
        sql.push_str(" AND t.needs_confirmation = 1 AND t.status NOT IN ('rejected','cancelled','completed')");
    } else if !f.statuses.is_empty() {
        let ph: Vec<String> = f.statuses.iter().map(|s| {
            args.push(Box::new(s.clone()));
            format!("?{}", args.len())
        }).collect();
        sql.push_str(&format!(" AND t.status IN ({})", ph.join(",")));
    } else if !f.include_closed {
        sql.push_str(" AND t.status NOT IN ('completed','cancelled','rejected')");
    } else {
        sql.push_str(" AND t.status <> 'rejected'");
    }
    if !f.search.trim().is_empty() {
        args.push(Box::new(format!("%{}%", f.search.trim())));
        let n = args.len();
        sql.push_str(&format!(" AND (t.title LIKE ?{n} OR t.description LIKE ?{n} OR p.name LIKE ?{n} OR t.people_json LIKE ?{n})"));
    }
    if let Some(m) = &f.meeting_id {
        args.push(Box::new(m.clone()));
        sql.push_str(&format!(" AND t.meeting_id = ?{}", args.len()));
    }
    if let Some(p) = &f.parent_id {
        args.push(Box::new(p.clone()));
        sql.push_str(&format!(" AND t.parent_id = ?{}", args.len()));
    } else if f.top_level_only {
        sql.push_str(" AND t.parent_id IS NULL");
    }
    if let Some(d) = &f.due_before {
        args.push(Box::new(ts(d)));
        sql.push_str(&format!(" AND t.due_at IS NOT NULL AND t.due_at <= ?{}", args.len()));
    }
    if let Some(tt) = &f.task_type {
        args.push(Box::new(tt.clone()));
        sql.push_str(&format!(" AND t.task_type = ?{}", args.len()));
    }
    sql.push_str(match f.sort.as_str() {
        "priority" => " ORDER BY t.priority ASC, (t.due_at IS NULL), t.due_at ASC",
        "created" => " ORDER BY t.created_at DESC",
        "updated" => " ORDER BY t.updated_at DESC",
        _ => " ORDER BY (t.due_at IS NULL), t.due_at ASC, t.priority ASC",
    });
    if let Some(l) = f.limit {
        sql.push_str(&format!(" LIMIT {}", l.clamp(1, 5000)));
    }
    let mut st = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let mut out: Vec<Task> = st.query_map(refs.as_slice(), row_to_task)?.collect::<rusqlite::Result<_>>()?;
    for t in out.iter_mut() {
        t.primary_source = primary_source(conn, &t.id);
    }
    Ok(out)
}

pub fn get(conn: &Connection, id: &str) -> Result<Task> {
    let mut t = conn
        .query_row(&format!("{SELECT} WHERE t.id = ?1"), [id], row_to_task)
        .optional()?
        .ok_or_else(|| anyhow!("task not found"))?;
    t.primary_source = primary_source(conn, id);
    Ok(t)
}

pub fn detail(conn: &Connection, id: &str) -> Result<TaskDetail> {
    let task = get(conn, id)?;
    let mut st = conn.prepare(
        "SELECT id, source_item_id, meeting_id, relation, evidence_quote, provider, url, source_label, added_at
         FROM task_sources WHERE task_id=?1 ORDER BY added_at",
    )?;
    let sources = st.query_map([id], source_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut st = conn.prepare(
        "SELECT id, kind, due_at, phrase, reason, created_at, superseded_at FROM task_deadlines WHERE task_id=?1 ORDER BY created_at",
    )?;
    let deadlines = st
        .query_map([id], |r| {
            Ok(Deadline {
                id: r.get(0)?,
                kind: r.get(1)?,
                due_at: opt_ts(r.get(2)?).unwrap_or_else(Utc::now),
                phrase: r.get(3)?,
                reason: r.get(4)?,
                created_at: opt_ts(r.get(5)?).unwrap_or_else(Utc::now),
                superseded_at: opt_ts(r.get(6)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut st = conn.prepare(
        "SELECT id, action, from_value, to_value, actor, reason, created_at, undone FROM audit_events
         WHERE entity_type='task' AND entity_id=?1 ORDER BY created_at DESC LIMIT 100",
    )?;
    let history = st
        .query_map([id], |r| {
            Ok(AuditEvent {
                id: r.get(0)?,
                action: r.get(1)?,
                from_value: r.get(2)?,
                to_value: r.get(3)?,
                actor: r.get(4)?,
                reason: r.get(5)?,
                created_at: opt_ts(r.get(6)?).unwrap_or_else(Utc::now),
                undone: r.get::<_, i64>(7)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let subtasks = list(conn, &TaskFilter { parent_id: Some(id.into()), include_closed: true, sort: "created".into(), ..Default::default() })?;
    let mut st = conn.prepare("SELECT depends_on FROM task_dependencies WHERE task_id=?1")?;
    let dep_ids = st.query_map([id], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let depends_on = dep_ids.iter().filter_map(|d| get(conn, d).ok()).collect();
    let possible_duplicate = task.possible_duplicate_of.as_deref().and_then(|d| get(conn, d).ok());
    Ok(TaskDetail { task, sources, deadlines, history, subtasks, depends_on, possible_duplicate })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NewTask {
    pub title: String,
    pub description: Option<String>,
    pub task_type: String,
    pub my_role: String,
    pub owner_name: Option<String>,
    pub status: String,
    pub priority: Option<i64>,
    pub priority_reason: Option<String>,
    pub confidence: f64,
    pub origin: String,
    pub needs_confirmation: bool,
    pub review_reason: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub due_kind: String,
    pub due_phrase: Option<String>,
    pub committed_at: Option<DateTime<Utc>>,
    pub meeting_id: Option<String>,
    pub project: Option<String>,
    pub expected_deliverable: Option<String>,
    pub completion_criteria: Option<String>,
    pub parent_id: Option<String>,
    pub recurrence: Option<String>,
    pub effort_minutes: Option<i64>,
    pub people: Vec<Person>,
    pub counterpart_email: Option<String>,
    pub possible_duplicate_of: Option<String>,
}

pub fn project_id(conn: &Connection, name: Option<&str>) -> Result<Option<String>> {
    let Some(name) = name.map(|n| n.trim()).filter(|n| !n.is_empty()) else { return Ok(None) };
    if let Some(id) = conn.query_row("SELECT id FROM projects WHERE name = ?1", [name], |r| r.get(0)).optional()? {
        return Ok(Some(id));
    }
    let id = new_id();
    conn.execute("INSERT INTO projects(id, name, created_at) VALUES (?1,?2,?3)", params![id, name, ts(&now())])?;
    Ok(Some(id))
}

pub fn create(conn: &Connection, t: &NewTask, actor: &str) -> Result<String> {
    if t.title.trim().is_empty() {
        bail!("title is required");
    }
    let st = if t.status.is_empty() { status::READY.to_string() } else { t.status.clone() };
    if !status::is_valid(&st) {
        bail!("invalid status {st}");
    }
    let id = new_id();
    let n = ts(&now());
    let pid = project_id(conn, t.project.as_deref())?;
    conn.execute(
        "INSERT INTO tasks(id, title, description, task_type, my_role, owner_name, status, priority, priority_reason,
            priority_locked, confidence, origin, needs_confirmation, review_reason, due_at, due_kind, due_phrase, committed_at, meeting_id,
            project_id, expected_deliverable, completion_criteria, parent_id, recurrence, effort_minutes, people_json,
            counterpart_email, fingerprint, possible_duplicate_of, last_update_at, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?30,?30)",
        params![
            id,
            text::truncate(t.title.trim(), 200),
            t.description,
            if t.task_type.is_empty() { "action" } else { &t.task_type },
            if t.my_role.is_empty() { "owner" } else { &t.my_role },
            t.owner_name,
            st,
            t.priority.unwrap_or(3).clamp(1, 4),
            t.priority_reason,
            t.priority.is_some() && actor == "user",
            t.confidence.clamp(0.0, 1.0),
            if t.origin.is_empty() { "manual" } else { &t.origin },
            t.needs_confirmation,
            t.review_reason,
            t.due_at.as_ref().map(ts),
            if t.due_at.is_some() { if t.due_kind.is_empty() { "confirmed" } else { &t.due_kind } } else { "none" },
            t.due_phrase,
            t.committed_at.as_ref().map(ts),
            t.meeting_id,
            pid,
            t.expected_deliverable,
            t.completion_criteria,
            t.parent_id,
            t.recurrence,
            t.effort_minutes,
            serde_json::to_string(&t.people)?,
            t.counterpart_email,
            text::fingerprint(&t.title),
            t.possible_duplicate_of,
            n,
        ],
    )?;
    if let Some(d) = t.due_at {
        let kind = if t.due_kind.is_empty() { "confirmed" } else { &t.due_kind };
        conn.execute(
            "INSERT INTO task_deadlines(id, task_id, kind, due_at, phrase, reason, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![new_id(), id, kind, ts(&d), t.due_phrase, "initial", n],
        )?;
    }
    audit(conn, "task", &id, "created", None, Some(&st), actor, &format!("origin={}", t.origin))?;
    Ok(id)
}

/// Distinguish "field absent" (None) from "field: null" (Some(None)) in JSON patches.
fn double_option<'de, D, T>(d: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub task_type: Option<String>,
    pub my_role: Option<String>,
    pub owner_name: Option<String>,
    pub priority: Option<i64>,
    /// Set (Some(Some)) or clear (Some(None)) the deadline. User-set deadlines are "confirmed".
    #[serde(deserialize_with = "double_option")]
    pub due_at: Option<Option<DateTime<Utc>>>,
    pub project: Option<String>,
    pub expected_deliverable: Option<String>,
    pub completion_criteria: Option<String>,
    #[serde(deserialize_with = "double_option")]
    pub meeting_id: Option<Option<String>>,
    pub effort_minutes: Option<i64>,
    #[serde(deserialize_with = "double_option")]
    pub recurrence: Option<Option<String>>,
    pub counterpart_email: Option<String>,
}

pub fn update(conn: &Connection, id: &str, p: &TaskPatch, actor: &str) -> Result<()> {
    let cur = get(conn, id)?;
    let n = ts(&now());
    crate::db::atomic(conn, |tx| {
    if let Some(v) = &p.title {
        if v.trim().is_empty() {
            bail!("title cannot be empty");
        }
        tx.execute("UPDATE tasks SET title=?1, fingerprint=?2 WHERE id=?3", params![v.trim(), text::fingerprint(v), id])?;
        audit(&tx, "task", id, "title", Some(&cur.title), Some(v), actor, "edited")?;
    }
    if let Some(v) = &p.description {
        tx.execute("UPDATE tasks SET description=?1 WHERE id=?2", params![v, id])?;
    }
    if let Some(v) = &p.task_type {
        tx.execute("UPDATE tasks SET task_type=?1 WHERE id=?2", params![v, id])?;
    }
    if let Some(v) = &p.my_role {
        if !["owner", "collaborator", "requester", "mentioned"].contains(&v.as_str()) {
            bail!("invalid role");
        }
        tx.execute("UPDATE tasks SET my_role=?1 WHERE id=?2", params![v, id])?;
        audit(&tx, "task", id, "owner", Some(&cur.my_role), Some(v), actor, "ownership changed")?;
    }
    if let Some(v) = &p.owner_name {
        tx.execute("UPDATE tasks SET owner_name=?1 WHERE id=?2", params![v, id])?;
    }
    if let Some(v) = p.priority {
        tx.execute("UPDATE tasks SET priority=?1, priority_locked=1, priority_reason='Set by you' WHERE id=?2", params![v.clamp(1, 4), id])?;
        audit(&tx, "task", id, "priority", Some(&cur.priority.to_string()), Some(&v.to_string()), actor, "")?;
    }
    if let Some(d) = &p.due_at {
        match d {
            Some(d) => {
                tx.execute("UPDATE task_deadlines SET superseded_at=?1 WHERE task_id=?2 AND superseded_at IS NULL", params![n, id])?;
                tx.execute(
                    "INSERT INTO task_deadlines(id, task_id, kind, due_at, reason, created_at) VALUES (?1,?2,'confirmed',?3,?4,?5)",
                    params![new_id(), id, ts(d), format!("set by {actor}"), n],
                )?;
                tx.execute("UPDATE tasks SET due_at=?1, due_kind='confirmed', conflict_note=NULL WHERE id=?2", params![ts(d), id])?;
            }
            None => {
                tx.execute("UPDATE task_deadlines SET superseded_at=?1 WHERE task_id=?2 AND superseded_at IS NULL", params![n, id])?;
                tx.execute("UPDATE tasks SET due_at=NULL, due_kind='none', due_phrase=NULL WHERE id=?1", params![id])?;
            }
        }
        audit(&tx, "task", id, "due", cur.due_at.as_ref().map(ts).as_deref(), d.as_ref().map(ts).as_deref(), actor, "deadline changed")?;
    }
    if let Some(v) = &p.project {
        let pid = project_id(&tx, Some(v))?;
        tx.execute("UPDATE tasks SET project_id=?1 WHERE id=?2", params![pid, id])?;
    }
    if let Some(v) = &p.expected_deliverable {
        tx.execute("UPDATE tasks SET expected_deliverable=?1 WHERE id=?2", params![v, id])?;
    }
    if let Some(v) = &p.completion_criteria {
        tx.execute("UPDATE tasks SET completion_criteria=?1 WHERE id=?2", params![v, id])?;
    }
    if let Some(v) = &p.meeting_id {
        tx.execute("UPDATE tasks SET meeting_id=?1 WHERE id=?2", params![v, id])?;
    }
    if let Some(v) = p.effort_minutes {
        tx.execute("UPDATE tasks SET effort_minutes=?1 WHERE id=?2", params![v.clamp(0, 6000), id])?;
    }
    if let Some(v) = &p.recurrence {
        if let Some(r) = v {
            if !["daily", "weekdays", "weekly", "monthly"].contains(&r.as_str()) {
                bail!("invalid recurrence");
            }
        }
        tx.execute("UPDATE tasks SET recurrence=?1 WHERE id=?2", params![v, id])?;
    }
    if let Some(v) = &p.counterpart_email {
        tx.execute("UPDATE tasks SET counterpart_email=?1 WHERE id=?2", params![v, id])?;
    }
    tx.execute("UPDATE tasks SET updated_at=?1, last_update_at=?1 WHERE id=?2", params![n, id])?;
    Ok(())
    })?;
    Ok(())
}

/// Change lifecycle status. Completion always records *why*.
pub fn set_status(conn: &Connection, id: &str, new_status: &str, actor: &str, reason: &str, evidence: Option<&str>) -> Result<()> {
    if !status::is_valid(new_status) {
        bail!("invalid status {new_status}");
    }
    let cur = get(conn, id)?;
    if cur.status == new_status {
        return Ok(());
    }
    let n = now();
    let evidence_text = match (new_status, evidence) {
        (status::COMPLETED, Some(e)) => Some(e.to_string()),
        (status::COMPLETED, None) if actor == "user" => Some(format!("Marked complete by you on {}", n.format("%Y-%m-%d %H:%M UTC"))),
        (status::COMPLETED, None) => bail!("automatic completion requires evidence"),
        _ => None,
    };
    crate::db::atomic(conn, |tx| {
    tx.execute(
        "UPDATE tasks SET status=?1, updated_at=?2, last_update_at=?2,
            completed_at = CASE WHEN ?1='completed' THEN ?2 ELSE NULL END,
            completion_evidence = CASE WHEN ?1='completed' THEN ?3 ELSE completion_evidence END,
            needs_confirmation = CASE WHEN ?1 IN ('completed','cancelled','rejected') THEN 0 ELSE needs_confirmation END
         WHERE id=?4",
        params![new_status, ts(&n), evidence_text, id],
    )?;
    audit(&tx, "task", id, "status", Some(&cur.status), Some(new_status), actor, if reason.is_empty() { evidence.unwrap_or("") } else { reason })?;
    if !status::is_open(new_status) {
        // Pending reminders for a closed task are obsolete — expire them (history is kept).
        tx.execute(
            "UPDATE notifications SET state='expired' WHERE entity_type='task' AND entity_id=?1 AND state IN ('pending','held','snoozed')",
            [id],
        )?;
        tx.execute("UPDATE commitments SET status='fulfilled' WHERE task_id=?1 AND status='open' AND ?2='completed'", params![id, new_status])?;
        tx.execute("UPDATE commitments SET status='dismissed' WHERE task_id=?1 AND status='open'", params![id])?;
    }
    Ok(())
    })?;
    if new_status == status::COMPLETED {
        if let Some(r) = &cur.recurrence {
            spawn_next_occurrence(conn, &cur, r)?;
        }
    }
    Ok(())
}

fn spawn_next_occurrence(conn: &Connection, t: &Task, rule: &str) -> Result<()> {
    let base = t.due_at.unwrap_or_else(now);
    let mut next = match rule {
        "daily" => base + Duration::days(1),
        "weekdays" => base + Duration::days(1),
        "weekly" => base + Duration::weeks(1),
        "monthly" => base + Duration::days(30),
        _ => return Ok(()),
    };
    if rule == "weekdays" {
        use chrono::Datelike;
        while next.weekday().num_days_from_monday() >= 5 {
            next += Duration::days(1);
        }
    }
    let id = create(
        conn,
        &NewTask {
            title: t.title.clone(),
            description: t.description.clone(),
            task_type: t.task_type.clone(),
            my_role: t.my_role.clone(),
            status: status::READY.into(),
            confidence: 1.0,
            origin: "manual".into(),
            due_at: Some(next),
            due_kind: "confirmed".into(),
            project: t.project.clone(),
            recurrence: Some(rule.into()),
            effort_minutes: t.effort_minutes,
            ..Default::default()
        },
        "agent",
    )?;
    audit(conn, "task", &id, "recurrence", Some(&t.id), None, "agent", "next occurrence")?;
    Ok(())
}

/// Accept a reviewed suggestion.
pub fn accept(conn: &Connection, id: &str) -> Result<()> {
    let cur = get(conn, id)?;
    let next = if matches!(cur.status.as_str(), status::DETECTED | status::NEEDS_CONFIRMATION) { status::READY } else { cur.status.as_str() };
    conn.execute(
        "UPDATE tasks SET needs_confirmation=0, review_reason=NULL, status=?1, confidence=MAX(confidence, 0.99), updated_at=?2 WHERE id=?3",
        params![next, ts(&now()), id],
    )?;
    audit(conn, "task", id, "accepted", Some(&cur.status), Some(next), "user", "accepted from review")?;
    Ok(())
}

/// Reject a false positive. Kept as 'rejected' (hidden) so the same source won't re-create it.
pub fn reject(conn: &Connection, id: &str, reason: &str) -> Result<()> {
    set_status(conn, id, status::REJECTED, "user", if reason.is_empty() { "rejected as not a task" } else { reason }, None)
}

/// Merge `dup` into `keep`: sources, deadline history and subtasks move; `dup` is cancelled.
pub fn merge(conn: &Connection, keep: &str, dup: &str, actor: &str) -> Result<()> {
    if keep == dup {
        bail!("cannot merge a task into itself");
    }
    let k = get(conn, keep)?;
    let d = get(conn, dup)?;
    crate::db::atomic(conn, |tx| {
    tx.execute("UPDATE task_sources SET task_id=?1 WHERE task_id=?2", params![keep, dup])?;
    tx.execute("UPDATE task_deadlines SET task_id=?1, superseded_at=COALESCE(superseded_at, ?3) WHERE task_id=?2", params![keep, dup, ts(&now())])?;
    tx.execute("UPDATE tasks SET parent_id=?1 WHERE parent_id=?2", params![keep, dup])?;
    tx.execute("UPDATE commitments SET task_id=?1 WHERE task_id=?2", params![keep, dup])?;
    tx.execute("UPDATE OR IGNORE task_dependencies SET task_id=?1 WHERE task_id=?2", params![keep, dup])?;
    // Keep the earliest *explicit/confirmed* deadline if the kept task has none.
    if k.due_at.is_none() && d.due_at.is_some() {
        tx.execute(
            "UPDATE tasks SET due_at=?1, due_kind=?2, due_phrase=?3 WHERE id=?4",
            params![d.due_at.as_ref().map(ts), d.due_kind, d.due_phrase, keep],
        )?;
    }
    tx.execute(
        "UPDATE tasks SET status='cancelled', duplicate_of=?1, needs_confirmation=0, updated_at=?2 WHERE id=?3",
        params![keep, ts(&now()), dup],
    )?;
    tx.execute("UPDATE tasks SET possible_duplicate_of=NULL, updated_at=?1 WHERE id IN (?2, ?3)", params![ts(&now()), keep, dup])?;
    tx.execute("UPDATE tasks SET possible_duplicate_of=NULL WHERE possible_duplicate_of=?1", params![dup])?;
    if !tx.query_row("SELECT 1 FROM tasks WHERE id=?1 AND review_reason LIKE 'Possible duplicate%'", [keep], |_| Ok(())).optional()?.is_none() {
        tx.execute("UPDATE tasks SET review_reason=NULL WHERE id=?1", [keep])?;
    }
    audit(&tx, "task", keep, "merged", Some(dup), Some(keep), actor, &format!("merged \"{}\"", d.title))?;
    audit(&tx, "task", dup, "merged_into", Some(&d.status), Some(keep), actor, &format!("merged into \"{}\"", k.title))?;
    Ok(())
    })?;
    Ok(())
}

/// Undo a status / due / owner / priority change recorded in the audit log.
pub fn undo(conn: &Connection, audit_id: &str) -> Result<()> {
    let (entity_id, action, from, undone): (String, String, Option<String>, i64) = conn.query_row(
        "SELECT entity_id, action, from_value, undone FROM audit_events WHERE id=?1 AND entity_type='task'",
        [audit_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    if undone != 0 {
        bail!("already undone");
    }
    match action.as_str() {
        "status" | "accepted" => {
            let from = from.ok_or_else(|| anyhow!("nothing to restore"))?;
            conn.execute(
                "UPDATE tasks SET status=?1, completed_at=NULL, needs_confirmation = CASE WHEN ?1 IN ('needs_confirmation','detected') THEN 1 ELSE needs_confirmation END, updated_at=?2 WHERE id=?3",
                params![from, ts(&now()), entity_id],
            )?;
        }
        "due" => {
            conn.execute(
                "UPDATE tasks SET due_at=?1, due_kind=CASE WHEN ?1 IS NULL THEN 'none' ELSE 'confirmed' END, updated_at=?2 WHERE id=?3",
                params![from, ts(&now()), entity_id],
            )?;
        }
        "owner" => {
            conn.execute("UPDATE tasks SET my_role=?1 WHERE id=?2", params![from, entity_id])?;
        }
        "priority" => {
            conn.execute("UPDATE tasks SET priority=?1 WHERE id=?2", params![from.and_then(|f| f.parse::<i64>().ok()).unwrap_or(3), entity_id])?;
        }
        "title" => {
            conn.execute("UPDATE tasks SET title=?1 WHERE id=?2", params![from, entity_id])?;
        }
        "merged_into" => {
            let from = from.unwrap_or_else(|| status::READY.into());
            conn.execute("UPDATE tasks SET status=?1, duplicate_of=NULL WHERE id=?2", params![from, entity_id])?;
        }
        _ => bail!("this change can't be undone automatically"),
    }
    conn.execute("UPDATE audit_events SET undone=1 WHERE id=?1", [audit_id])?;
    audit(conn, "task", &entity_id, "undo", None, Some(&action), "user", &format!("undid {audit_id}"))?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM tasks WHERE id=?1", [id])?;
    audit(conn, "task", id, "deleted", None, None, "user", "")?;
    Ok(())
}

pub struct SourceLink<'a> {
    pub source_item_id: Option<&'a str>,
    pub meeting_id: Option<&'a str>,
    pub relation: &'a str,
    pub quote: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub url: Option<&'a str>,
    pub label: Option<String>,
}

pub fn add_source(conn: &Connection, task_id: &str, s: &SourceLink) -> Result<()> {
    // Same source + relation only once.
    if let Some(sid) = s.source_item_id {
        let exists = conn
            .query_row("SELECT 1 FROM task_sources WHERE task_id=?1 AND source_item_id=?2 AND relation=?3", params![task_id, sid, s.relation], |_| Ok(()))
            .optional()?;
        if exists.is_some() {
            return Ok(());
        }
    }
    conn.execute(
        "INSERT INTO task_sources(id, task_id, source_item_id, meeting_id, relation, evidence_quote, provider, url, source_label, added_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![new_id(), task_id, s.source_item_id, s.meeting_id, s.relation, s.quote.map(|q| text::truncate(q, 400)), s.provider, s.url, s.label, ts(&now())],
    )?;
    Ok(())
}

fn kind_rank(k: &str) -> i32 {
    match k {
        "confirmed" => 4,
        "explicit" => 3,
        "inferred" => 2,
        "suggested" => 1,
        _ => 0,
    }
}

#[derive(Debug, PartialEq)]
pub enum DeadlineOutcome {
    Set,
    Changed { from: DateTime<Utc>, to: DateTime<Utc> },
    Unchanged,
    Conflict(String),
    SuggestedOnly,
}

/// Apply a deadline observed in a source, respecting precedence:
/// confirmed (by user) > explicit > inferred > suggested. Suggestions never become `due_at`.
pub fn apply_deadline(
    conn: &Connection,
    task_id: &str,
    kind: &str,
    due: DateTime<Utc>,
    phrase: Option<&str>,
    source_item_id: Option<&str>,
    observed_at: DateTime<Utc>,
    author_is_counterpart: bool,
) -> Result<DeadlineOutcome> {
    let t = get(conn, task_id)?;
    let n = ts(&now());
    let insert = |superseded: Option<&str>, reason: &str| -> Result<()> {
        conn.execute(
            "INSERT INTO task_deadlines(id, task_id, kind, due_at, phrase, source_item_id, reason, created_at, superseded_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![new_id(), task_id, kind, ts(&due), phrase, source_item_id, reason, n, superseded],
        )?;
        Ok(())
    };
    if kind == "suggested" {
        let has = conn.query_row("SELECT 1 FROM task_deadlines WHERE task_id=?1 AND kind='suggested' AND superseded_at IS NULL", [task_id], |_| Ok(())).optional()?;
        if has.is_none() && t.due_at.is_none() {
            insert(None, "suggested — needs your confirmation")?;
        }
        return Ok(DeadlineOutcome::SuggestedOnly);
    }
    let Some(cur) = t.due_at else {
        insert(None, "from source")?;
        conn.execute("UPDATE tasks SET due_at=?1, due_kind=?2, due_phrase=?3, updated_at=?4 WHERE id=?5", params![ts(&due), kind, phrase, n, task_id])?;
        conn.execute("UPDATE task_deadlines SET superseded_at=?1 WHERE task_id=?2 AND kind='suggested' AND superseded_at IS NULL", params![n, task_id])?;
        return Ok(DeadlineOutcome::Set);
    };
    if (cur - due).num_minutes().abs() < 60 {
        return Ok(DeadlineOutcome::Unchanged);
    }
    let cur_rank = kind_rank(&t.due_kind);
    let new_rank = kind_rank(kind);
    // When was the *current* deadline stated? (source message time, not processing time)
    let last_change: DateTime<Utc> = conn
        .query_row(
            "SELECT COALESCE(si.sent_at, t.committed_at, d.created_at) FROM task_deadlines d JOIN tasks t ON t.id = d.task_id LEFT JOIN source_items si ON si.id = d.source_item_id
             WHERE d.task_id=?1 AND d.superseded_at IS NULL AND d.kind <> 'suggested' ORDER BY d.created_at DESC LIMIT 1",
            [task_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|s| parse_ts(&s))
        .or(t.committed_at)
        .unwrap_or(t.created_at);
    if t.due_kind == "confirmed" || (cur_rank == 3 && new_rank == 3 && !author_is_counterpart) {
        let note = format!(
            "A source suggests {} ({}) but the current deadline is {}. Kept did not change it.",
            due.format("%a %d %b %H:%M UTC"),
            phrase.unwrap_or(kind),
            cur.format("%a %d %b %H:%M UTC")
        );
        insert(Some(&n), "conflicting — not applied")?;
        conn.execute("UPDATE tasks SET conflict_note=?1, needs_confirmation=1, review_reason='Conflicting deadline', updated_at=?2 WHERE id=?3", params![note, n, task_id])?;
        return Ok(DeadlineOutcome::Conflict(note));
    }
    if new_rank > cur_rank || (new_rank == cur_rank && observed_at >= last_change - Duration::minutes(1)) {
        conn.execute("UPDATE task_deadlines SET superseded_at=?1 WHERE task_id=?2 AND superseded_at IS NULL", params![n, task_id])?;
        insert(None, "updated by newer source")?;
        conn.execute(
            "UPDATE tasks SET due_at=?1, due_kind=?2, due_phrase=?3, updated_at=?4, last_update_at=?4 WHERE id=?5",
            params![ts(&due), kind, phrase, n, task_id],
        )?;
        audit(conn, "task", task_id, "due", Some(&ts(&cur)), Some(&ts(&due)), "agent", &format!("deadline updated from source ({})", phrase.unwrap_or(kind)))?;
        return Ok(DeadlineOutcome::Changed { from: cur, to: due });
    }
    insert(Some(&n), "older or weaker than current deadline")?;
    Ok(DeadlineOutcome::Unchanged)
}

/// Candidate duplicate among open tasks. Returns (task_id, score, strong).
pub fn find_duplicate(conn: &Connection, title: &str, thread_key: Option<&str>, counterpart: Option<&str>, exclude: Option<&str>) -> Result<Option<(String, f64, bool)>> {
    let mut st = conn.prepare(
        "SELECT t.id, t.title, t.counterpart_email,
            (SELECT GROUP_CONCAT(si.thread_key, '|') FROM task_sources ts JOIN source_items si ON si.id = ts.source_item_id WHERE ts.task_id = t.id)
         FROM tasks t WHERE t.status NOT IN ('completed','cancelled','rejected') AND t.duplicate_of IS NULL",
    )?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?)))?;
    let mut best: Option<(String, f64, bool)> = None;
    for row in rows {
        let (id, t, cp, threads) = row?;
        if Some(id.as_str()) == exclude {
            continue;
        }
        // Names make unrelated tasks look alike ("Send Sara X" vs "Send Sara Y"): ignore them.
        let mut names: Vec<String> = vec![];
        if let Some(c) = counterpart {
            names.push(c.split('@').next().unwrap_or("").split(['.', '_', '-']).next().unwrap_or("").to_lowercase());
        }
        if let Some(c) = &cp {
            names.push(c.split('@').next().unwrap_or("").split(['.', '_', '-']).next().unwrap_or("").to_lowercase());
        }
        let strip = |s: &str| -> String {
            s.split_whitespace()
                .filter(|w| {
                    let l = w.to_lowercase();
                    let l = l.trim_end_matches("'s").trim_matches(|c: char| !c.is_alphanumeric());
                    !names.iter().any(|n| !n.is_empty() && n == l)
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        let (a, b) = (strip(title), strip(&t));
        // A single shared word ("report") is not enough to call two tasks the same.
        let shared = text::keywords(&a).intersection(&text::keywords(&b)).count();
        let ov = if shared >= 2 { text::overlap(&a, &b) } else { 0.0 };
        let jac = text::similarity(&a, &b);
        let same_thread = matches!((thread_key, &threads), (Some(k), Some(ts)) if ts.split('|').any(|x| x == k));
        let same_cp = matches!((counterpart, &cp), (Some(a), Some(b)) if a.eq_ignore_ascii_case(b));
        let score = jac.max(ov * 0.85) + if same_thread { 0.25 } else { 0.0 } + if same_cp { 0.1 } else { 0.0 };
        let strong = (same_thread && ov >= 0.5) || (same_cp && ov >= 0.75 && jac >= 0.5) || jac >= 0.8;
        if score >= 0.5 && best.as_ref().map(|b| score > b.1).unwrap_or(true) {
            best = Some((id, score, strong));
        }
    }
    Ok(best)
}

pub fn add_dependency(conn: &Connection, task_id: &str, depends_on: &str, note: Option<&str>) -> Result<()> {
    if task_id == depends_on {
        bail!("a task cannot depend on itself");
    }
    // Prevent trivial cycles.
    let reverse = conn.query_row("SELECT 1 FROM task_dependencies WHERE task_id=?1 AND depends_on=?2", params![depends_on, task_id], |_| Ok(())).optional()?;
    if reverse.is_some() {
        bail!("that would create a dependency cycle");
    }
    conn.execute("INSERT OR IGNORE INTO task_dependencies(task_id, depends_on, note) VALUES (?1,?2,?3)", params![task_id, depends_on, note])?;
    Ok(())
}

pub fn remove_dependency(conn: &Connection, task_id: &str, depends_on: &str) -> Result<()> {
    conn.execute("DELETE FROM task_dependencies WHERE task_id=?1 AND depends_on=?2", params![task_id, depends_on])?;
    Ok(())
}

pub fn parse_due(s: &str) -> Option<DateTime<Utc>> {
    parse_ts(s)
}
