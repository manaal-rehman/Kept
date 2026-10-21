//! User preferences (one JSON document in the `preferences` table).
//! Every default here is user-configurable from Settings.

use anyhow::Result;
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use chrono_tz::Tz;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    /// none | anthropic | openai | openai_compatible
    pub provider: String,
    pub model: String,
    /// Only for openai_compatible (e.g. http://localhost:11434/v1 for Ollama).
    pub base_url: String,
    pub process_email: bool,
    pub process_chat: bool,
    pub process_calendar: bool,
    pub redact_pii: bool,
    pub max_chars_per_request: usize,
    pub daily_request_limit: u32,
    pub timeout_seconds: u64,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: "none".into(),
            model: String::new(),
            base_url: String::new(),
            process_email: true,
            process_chat: true,
            process_calendar: true,
            redact_pii: true,
            max_chars_per_request: 6000,
            daily_request_limit: 300,
            timeout_seconds: 45,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub onboarded: bool,
    pub user_name: String,
    /// Every address/handle that is "me". Used for ownership decisions.
    pub user_emails: Vec<String>,
    pub user_aliases: Vec<String>,
    pub timezone: String,
    /// ISO weekday numbers, 1 = Monday .. 7 = Sunday.
    pub working_days: Vec<u32>,
    pub work_start: String,
    pub work_end: String,
    pub holidays: Vec<NaiveDate>,
    pub quiet_hours_enabled: bool,
    pub quiet_start: String,
    pub quiet_end: String,
    /// balanced | deadline_first | quick_wins
    pub planning_style: String,
    pub monitoring_paused: bool,
    pub notifications_paused_until: Option<DateTime<Utc>>,
    pub sync_interval_minutes: u32,
    pub max_notifications_per_hour: u32,
    pub digest_enabled: bool,
    pub digest_time: String,
    pub eod_review_enabled: bool,
    pub eod_review_time: String,
    pub meeting_prep_buffer_minutes: i64,
    pub vip_emails: Vec<String>,
    /// auto = AI when configured, otherwise built-in rules; rules_only = never call AI.
    pub extraction_mode: String,
    /// Tasks at or above this confidence (and clearly mine) skip the review queue.
    pub auto_accept_threshold: f64,
    pub review_everything: bool,
    pub retention_body_days: u32,
    pub retention_item_days: u32,
    pub autostart: bool,
    pub start_minimized: bool,
    pub close_to_tray: bool,
    pub initial_sync_days: u32,
    pub google_client_id: String,
    pub microsoft_client_id: String,
    pub microsoft_tenant: String,
    pub slack_client_id: String,
    pub ai: AiSettings,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            onboarded: false,
            user_name: String::new(),
            user_emails: vec![],
            user_aliases: vec![],
            timezone: "UTC".into(),
            working_days: vec![1, 2, 3, 4, 5],
            work_start: "09:00".into(),
            work_end: "17:30".into(),
            holidays: vec![],
            quiet_hours_enabled: true,
            quiet_start: "21:00".into(),
            quiet_end: "07:30".into(),
            planning_style: "balanced".into(),
            monitoring_paused: false,
            notifications_paused_until: None,
            sync_interval_minutes: 10,
            max_notifications_per_hour: 6,
            digest_enabled: true,
            digest_time: "08:30".into(),
            eod_review_enabled: false,
            eod_review_time: "17:00".into(),
            meeting_prep_buffer_minutes: 30,
            vip_emails: vec![],
            extraction_mode: "auto".into(),
            auto_accept_threshold: 0.85,
            review_everything: false,
            retention_body_days: 30,
            retention_item_days: 180,
            autostart: false,
            start_minimized: false,
            close_to_tray: true,
            initial_sync_days: 14,
            google_client_id: String::new(),
            microsoft_client_id: String::new(),
            microsoft_tenant: "common".into(),
            slack_client_id: String::new(),
            ai: AiSettings::default(),
        }
    }
}

impl Preferences {
    pub fn tz(&self) -> Tz {
        self.timezone.parse().unwrap_or(chrono_tz::UTC)
    }
    pub fn is_me(&self, email: &str) -> bool {
        let e = email.trim().to_lowercase();
        !e.is_empty() && self.user_emails.iter().any(|m| m.trim().to_lowercase() == e)
    }
    pub fn first_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .user_name
            .split_whitespace()
            .next()
            .map(|s| vec![s.to_lowercase()])
            .unwrap_or_default();
        v.extend(self.user_aliases.iter().map(|a| a.trim_start_matches('@').to_lowercase()));
        v.retain(|s| s.len() >= 2);
        v
    }
    pub fn validate(&mut self) {
        if self.timezone.parse::<Tz>().is_err() {
            self.timezone = "UTC".into();
        }
        for t in [&mut self.work_start, &mut self.work_end, &mut self.quiet_start, &mut self.quiet_end, &mut self.digest_time, &mut self.eod_review_time] {
            if parse_hm(t).is_none() {
                *t = "09:00".into();
            }
        }
        self.working_days.retain(|d| (1..=7).contains(d));
        self.sync_interval_minutes = self.sync_interval_minutes.clamp(2, 240);
        self.max_notifications_per_hour = self.max_notifications_per_hour.clamp(1, 60);
        self.auto_accept_threshold = self.auto_accept_threshold.clamp(0.5, 1.0);
        self.ai.max_chars_per_request = self.ai.max_chars_per_request.clamp(1000, 40000);
        self.initial_sync_days = self.initial_sync_days.clamp(1, 90);
        self.user_emails = self.user_emails.iter().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).collect();
        self.user_emails.dedup();
    }
}

pub fn parse_hm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").ok()
}

pub fn load(conn: &Connection) -> Result<Preferences> {
    let v: Option<String> = conn
        .query_row("SELECT value_json FROM preferences WHERE key='settings'", [], |r| r.get(0))
        .optional()?;
    Ok(v.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
}

pub fn save(conn: &Connection, p: &Preferences) -> Result<()> {
    let mut p = p.clone();
    p.validate();
    conn.execute(
        "INSERT INTO preferences(key, value_json) VALUES('settings', ?1)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
        params![serde_json::to_string(&p)?],
    )?;
    Ok(())
}

/// Small key/value state used by the scheduler (last digest date etc.).
pub fn get_state(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value_json FROM preferences WHERE key=?1", [format!("state:{key}")], |r| r.get(0))
        .optional()?)
}

pub fn set_state(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO preferences(key, value_json) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
        params![format!("state:{key}"), value],
    )?;
    Ok(())
}
