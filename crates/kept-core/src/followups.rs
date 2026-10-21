//! Follow-ups view: replies owed, promises to contact people, things you're waiting on,
//! approvals pending, dependencies on others, and promises without completion evidence.

use crate::db::{now, opt_ts};
use crate::models::{Commitment, Task};
use crate::tasks::{self, TaskFilter};
use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Followups {
    pub awaiting_my_reply: Vec<Task>,
    pub people_to_contact: Vec<Task>,
    pub waiting_on_others: Vec<Commitment>,
    pub waiting_tasks: Vec<Task>,
    pub awaiting_approval: Vec<Task>,
    pub blocked_by_others: Vec<Task>,
    pub promises_without_evidence: Vec<Task>,
}

pub fn list(conn: &Connection) -> Result<Followups> {
    let open = tasks::list(conn, &TaskFilter::default())?;
    let active = |t: &&Task| !t.needs_confirmation;
    let mut st = conn.prepare(
        "SELECT c.id, c.source_item_id, c.direction, c.who_name, c.who_email, c.text, c.due_at, c.status, c.task_id, c.created_at,
                si.sender_name || ' · ' || COALESCE(si.subject, ''), si.url
         FROM commitments c LEFT JOIN source_items si ON si.id = c.source_item_id
         WHERE c.direction='theirs' AND c.status='open' ORDER BY COALESCE(c.due_at, c.created_at)",
    )?;
    let waiting_on_others = st
        .query_map([], |r| {
            Ok(Commitment {
                id: r.get(0)?,
                source_item_id: r.get(1)?,
                direction: r.get(2)?,
                who_name: r.get(3)?,
                who_email: r.get(4)?,
                text: r.get(5)?,
                due_at: opt_ts(r.get(6)?),
                status: r.get(7)?,
                task_id: r.get(8)?,
                created_at: opt_ts(r.get(9)?).unwrap_or_else(Utc::now),
                source_label: r.get(10)?,
                source_url: r.get(11)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let n = now();
    Ok(Followups {
        awaiting_my_reply: open.iter().filter(active).filter(|t| t.task_type == "reply").cloned().collect(),
        people_to_contact: open.iter().filter(active).filter(|t| t.task_type == "followup" && t.status != "awaiting_approval").cloned().collect(),
        waiting_on_others,
        waiting_tasks: open.iter().filter(active).filter(|t| t.status == "waiting").cloned().collect(),
        awaiting_approval: open.iter().filter(|t| matches!(t.status.as_str(), "awaiting_approval" | "sent")).cloned().collect(),
        blocked_by_others: open.iter().filter(active).filter(|t| t.status == "blocked").cloned().collect(),
        promises_without_evidence: open
            .iter()
            .filter(active)
            .filter(|t| t.origin != "manual" && t.my_role == "owner" && t.due_at.map(|d| d < n).unwrap_or(false) && t.completion_evidence.is_none())
            .cloned()
            .collect(),
    })
}

pub fn set_commitment_status(conn: &Connection, id: &str, status: &str) -> Result<()> {
    anyhow::ensure!(matches!(status, "open" | "fulfilled" | "dismissed"), "invalid status");
    conn.execute("UPDATE commitments SET status=?1 WHERE id=?2", rusqlite::params![status, id])?;
    crate::db::audit(conn, "commitment", id, "status", None, Some(status), "user", "")?;
    Ok(())
}
