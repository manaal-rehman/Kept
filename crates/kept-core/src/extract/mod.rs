//! Commitment & action-item extraction.
//!
//! Two interchangeable extractors produce the same `Extracted` items:
//! * `rules`  — deterministic, offline, always available (and the fallback).
//! * `ai`     — a hosted / local LLM through `crate::ai`, validated against a schema
//!              and grounded: every item must quote text that really exists in the source.
//!
//! Extractors only *propose*. `crate::ingest` decides what becomes a task, a review
//! suggestion, a status update, or nothing.

pub mod ai;
pub mod rules;

use crate::dates::DeadlineGuess;
use crate::models::{Person, SourceItem};
use crate::prefs::Preferences;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What kind of signal a sentence carries.
pub mod kind {
    /// I promised something ("I'll send the deck Friday").
    pub const MY_COMMITMENT: &str = "my_commitment";
    /// Someone asked me to do something.
    pub const REQUEST_TO_ME: &str = "request_to_me";
    /// Someone needs my approval / decision.
    pub const APPROVAL_REQUEST: &str = "approval_request";
    /// Context shows I own something nobody explicitly asked for.
    pub const IMPLICIT_ASSIGNMENT: &str = "implicit_assignment";
    /// A direct question that needs my reply.
    pub const REPLY_NEEDED: &str = "reply_needed";
    /// Someone chases me about an earlier promise.
    pub const FOLLOWUP_NUDGE: &str = "followup_nudge";
    /// Someone else promised something to me (waiting-for, never my task).
    pub const THEIR_COMMITMENT: &str = "their_commitment";
    /// I (apparently) did the thing: sent, shared, submitted.
    pub const COMPLETION_EVIDENCE: &str = "completion_evidence";
    /// Someone approved my deliverable.
    pub const APPROVAL_RECEIVED: &str = "approval_received";

    pub const ALL: &[&str] = &[
        MY_COMMITMENT, REQUEST_TO_ME, APPROVAL_REQUEST, IMPLICIT_ASSIGNMENT, REPLY_NEEDED, FOLLOWUP_NUDGE,
        THEIR_COMMITMENT, COMPLETION_EVIDENCE, APPROVAL_RECEIVED,
    ];
    pub fn creates_task(k: &str) -> bool {
        matches!(k, MY_COMMITMENT | REQUEST_TO_ME | APPROVAL_REQUEST | IMPLICIT_ASSIGNMENT | REPLY_NEEDED | FOLLOWUP_NUDGE)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Extracted {
    pub kind: String,
    pub title: String,
    pub description: Option<String>,
    /// action | reply | approval | deliverable | followup | prep | review
    pub task_type: String,
    /// owner | collaborator | requester | mentioned
    pub my_role: String,
    pub owner_name: Option<String>,
    /// Exact text from the source that justifies the item.
    pub evidence: String,
    pub due_phrase: Option<String>,
    pub due: Option<DeadlineGuess>,
    pub confidence: f64,
    pub needs_confirmation: bool,
    pub ambiguity: Option<String>,
    pub expected_deliverable: Option<String>,
    pub completion_criteria: Option<String>,
    pub counterpart: Option<Person>,
    pub project: Option<String>,
    /// For updates / evidence / nudges: an existing open task this refers to.
    pub related_task_id: Option<String>,
    /// For completion evidence: sent | draft_sent | submitted | done
    pub completion_stage: Option<String>,
    pub urgent: bool,
    /// rules | ai
    pub extractor: String,
}

/// Everything an extractor may look at for one message.
pub struct ExtractContext<'a> {
    pub prefs: &'a Preferences,
    pub item: &'a SourceItem,
    /// Earlier messages in the same thread, oldest first (bounded).
    pub thread: &'a [SourceItem],
    /// Open tasks the item might relate to: (id, title).
    pub open_tasks: &'a [(String, String)],
    pub now: DateTime<Utc>,
}

impl ExtractContext<'_> {
    /// The newly written part of the message (quoted history removed).
    pub fn new_text(&self) -> String {
        let body = self.item.body_text.clone().unwrap_or_default();
        if self.item.kind == "email" {
            crate::text::strip_quoted(&body)
        } else {
            body
        }
    }

    pub fn sender_first_name(&self) -> String {
        self.item
            .sender_name
            .as_deref()
            .and_then(|n| n.split_whitespace().next())
            .map(|s| s.to_string())
            .or_else(|| self.item.sender_email.as_deref().map(|e| e.split('@').next().unwrap_or(e).to_string()))
            .unwrap_or_else(|| "them".into())
    }

    /// Am I a direct (To) recipient? For chat, DMs list me in `to`; channel posts don't.
    pub fn addressed_to_me(&self) -> bool {
        self.item.to.iter().any(|a| self.prefs.is_me(&crate::text::parse_address(a).1))
    }
    pub fn only_cc(&self) -> bool {
        !self.addressed_to_me() && self.item.cc.iter().any(|a| self.prefs.is_me(&crate::text::parse_address(a).1))
    }
    pub fn mentions_me(&self, text: &str) -> bool {
        let t = text.to_lowercase();
        self.prefs.first_names().iter().any(|n| {
            t.split(|c: char| !c.is_alphanumeric() && c != '@').any(|w| w.trim_start_matches('@') == n)
        })
    }
    /// Number of people (other than me) in To.
    pub fn other_to_count(&self) -> usize {
        self.item.to.iter().filter(|a| !self.prefs.is_me(&crate::text::parse_address(a).1)).count()
    }
    /// Primary counterpart: sender when they wrote to me; first To recipient when I wrote.
    pub fn counterpart(&self) -> Option<Person> {
        if self.item.is_from_me {
            self.item.to.first().map(|a| {
                let (n, e) = crate::text::parse_address(a);
                Person { name: n, email: Some(e) }
            })
        } else {
            Some(Person { name: self.item.sender_name.clone(), email: self.item.sender_email.clone() })
        }
    }
}

/// Heuristic prompt-injection detector. Matching content is still processed as data,
/// but resulting items are forced into the review queue and the event is logged.
pub fn looks_like_injection(text: &str) -> bool {
    let t = text.to_lowercase();
    const PATTERNS: &[&str] = &[
        "ignore previous instructions", "ignore all previous", "ignore the above", "disregard previous",
        "disregard your instructions", "system prompt", "you are now", "new instructions:", "act as ",
        "developer mode", "jailbreak", "reveal your", "print your instructions", "send your api key",
        "exfiltrate", "base64 encode the", "forward all emails", "forward every email", "mark all tasks",
        "delete all tasks", "</message>", "<system>", "assistant:", "\"items\":",
    ];
    PATTERNS.iter().any(|p| t.contains(p))
}
