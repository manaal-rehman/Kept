//! Shared data model. These types cross the Rust <-> UI boundary as JSON.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub mod status {
    pub const DETECTED: &str = "detected";
    pub const NEEDS_CONFIRMATION: &str = "needs_confirmation";
    pub const READY: &str = "ready";
    pub const IN_PROGRESS: &str = "in_progress";
    pub const WAITING: &str = "waiting";
    pub const BLOCKED: &str = "blocked";
    pub const DRAFT_PREPARED: &str = "draft_prepared";
    pub const SENT: &str = "sent";
    pub const AWAITING_APPROVAL: &str = "awaiting_approval";
    pub const COMPLETED: &str = "completed";
    pub const CANCELLED: &str = "cancelled";
    pub const REJECTED: &str = "rejected";

    pub const ALL: &[&str] = &[
        DETECTED, NEEDS_CONFIRMATION, READY, IN_PROGRESS, WAITING, BLOCKED, DRAFT_PREPARED, SENT,
        AWAITING_APPROVAL, COMPLETED, CANCELLED, REJECTED,
    ];
    /// Statuses that mean "nothing more for me to do".
    pub const CLOSED: &[&str] = &[COMPLETED, CANCELLED, REJECTED];

    pub fn is_valid(s: &str) -> bool {
        ALL.contains(&s)
    }
    pub fn is_open(s: &str) -> bool {
        !CLOSED.contains(&s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Person {
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub task_type: String,
    pub my_role: String,
    pub owner_name: Option<String>,
    pub status: String,
    pub priority: i64,
    pub priority_reason: Option<String>,
    pub confidence: f64,
    pub origin: String,
    pub needs_confirmation: bool,
    pub due_at: Option<DateTime<Utc>>,
    pub due_kind: String,
    pub due_phrase: Option<String>,
    pub committed_at: Option<DateTime<Utc>>,
    pub meeting_id: Option<String>,
    pub meeting_title: Option<String>,
    pub project: Option<String>,
    pub expected_deliverable: Option<String>,
    pub completion_criteria: Option<String>,
    pub parent_id: Option<String>,
    pub recurrence: Option<String>,
    pub effort_minutes: Option<i64>,
    pub people: Vec<Person>,
    pub counterpart_email: Option<String>,
    pub possible_duplicate_of: Option<String>,
    pub conflict_note: Option<String>,
    pub review_reason: Option<String>,
    pub completion_evidence: Option<String>,
    pub last_update_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub source_count: i64,
    pub primary_source: Option<SourceRef>,
    pub subtask_count: i64,
    pub subtasks_done: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRef {
    pub id: String,
    pub source_item_id: Option<String>,
    pub meeting_id: Option<String>,
    pub relation: String,
    pub evidence_quote: Option<String>,
    pub provider: Option<String>,
    pub url: Option<String>,
    pub label: Option<String>,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deadline {
    pub id: String,
    pub kind: String,
    pub due_at: DateTime<Utc>,
    pub phrase: Option<String>,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub superseded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: String,
    pub action: String,
    pub from_value: Option<String>,
    pub to_value: Option<String>,
    pub actor: String,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub undone: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDetail {
    pub task: Task,
    pub sources: Vec<SourceRef>,
    pub deadlines: Vec<Deadline>,
    pub history: Vec<AuditEvent>,
    pub subtasks: Vec<Task>,
    pub depends_on: Vec<Task>,
    pub possible_duplicate: Option<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceItem {
    pub id: String,
    pub account_id: String,
    pub provider: String,
    pub kind: String,
    pub external_id: String,
    pub thread_key: Option<String>,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: Option<String>,
    pub body_text: Option<String>,
    pub snippet: Option<String>,
    pub url: Option<String>,
    pub sent_at: DateTime<Utc>,
    pub is_from_me: bool,
    pub process_state: String,
    pub process_note: Option<String>,
}

/// What a connector produces before it is stored. Provider-agnostic.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IncomingMessage {
    pub external_id: String,
    pub kind: String, // email | chat
    pub thread_key: Option<String>,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: Option<String>,
    pub body_text: String,
    pub url: Option<String>,
    pub sent_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IncomingEvent {
    pub external_id: String,
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub timezone: Option<String>,
    pub all_day: bool,
    pub organizer_email: Option<String>,
    pub organizer_name: Option<String>,
    pub attendees: Vec<Attendee>,
    pub join_url: Option<String>,
    pub url: Option<String>,
    pub status: String,
    pub my_response: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Attendee {
    pub email: String,
    pub name: Option<String>,
    pub response: Option<String>,
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meeting {
    pub id: String,
    pub account_id: String,
    pub title: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub timezone: Option<String>,
    pub all_day: bool,
    pub organizer_email: Option<String>,
    pub organizer_name: Option<String>,
    pub join_url: Option<String>,
    pub url: Option<String>,
    pub status: String,
    pub my_response: Option<String>,
    pub prep_state: String,
    pub attendees: Vec<Attendee>,
    pub open_task_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub provider: String,
    pub category: String,
    pub display_name: String,
    pub email: Option<String>,
    pub status: String,
    pub status_detail: Option<String>,
    pub scopes: Option<String>,
    pub enabled_streams: Vec<String>,
    pub connected_at: Option<DateTime<Utc>>,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub next_sync_at: Option<DateTime<Utc>>,
    pub backoff_until: Option<DateTime<Utc>>,
    pub error_count: i64,
    pub items_synced: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub fingerprint: String,
    pub category: String,
    pub priority: i64,
    pub title: String,
    pub body: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub group_key: Option<String>,
    pub state: String,
    pub deliver_after: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub delivery_error: Option<String>,
    pub read_at: Option<DateTime<Utc>>,
    pub snoozed_until: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationRule {
    pub category: String,
    pub enabled: bool,
    pub desktop: bool,
    pub lead_minutes: i64,
    pub repeat_minutes: i64,
    pub min_priority: i64,
    pub bypass_quiet: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub id: String,
    pub source_item_id: Option<String>,
    pub direction: String,
    pub who_name: Option<String>,
    pub who_email: Option<String>,
    pub text: String,
    pub due_at: Option<DateTime<Utc>>,
    pub status: String,
    pub task_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub source_label: Option<String>,
    pub source_url: Option<String>,
}

/// A toast the shell should show now. Produced by the notification engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Toast {
    pub notification_id: String,
    pub title: String,
    pub body: String,
    pub priority: i64,
}

/// File metadata from a document store (no file contents are fetched).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IncomingDocument {
    pub external_id: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub url: Option<String>,
    pub modified_at: Option<DateTime<Utc>>,
    pub modified_by: Option<String>,
    pub owner: Option<String>,
}

/// A task assigned to the user in an external tool.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IncomingExternalTask {
    pub external_id: String,
    pub key: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub is_done: bool,
    pub due_at: Option<DateTime<Utc>>,
    pub due_is_date_only: bool,
    pub url: Option<String>,
    pub project: Option<String>,
    pub priority: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub url: Option<String>,
    pub modified_at: Option<DateTime<Utc>>,
    pub modified_by: Option<String>,
}
