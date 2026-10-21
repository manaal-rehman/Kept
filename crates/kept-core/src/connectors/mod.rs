//! Provider-agnostic connector layer.
//!
//! Every connector maps provider data into the common model (`IncomingMessage`,
//! `IncomingEvent`) and reports capabilities, so the rest of Kept never needs to know
//! which provider data came from. Adding a provider = add a module + a `ProviderInfo`.

pub mod docs;
pub mod files;
pub mod google;
pub mod slack;
pub mod teams;
pub mod worktools;
pub mod http;
pub mod microsoft;
pub mod oauth;
pub mod sample;

use crate::models::{IncomingDocument, IncomingEvent, IncomingExternalTask, IncomingMessage};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    #[error("authorization expired or revoked — reconnect required")]
    Auth,
    #[error("rate limited (retry after {0}s)")]
    RateLimited(u64),
    #[error("temporary problem: {0}")]
    Transient(String),
    #[error("{0}")]
    Permanent(String),
    #[error("sync cursor is no longer valid")]
    CursorExpired,
}

impl ConnectorError {
    pub fn status(&self) -> &'static str {
        match self {
            ConnectorError::Auth => "attention",
            ConnectorError::Permanent(_) => "error",
            _ => "connected",
        }
    }
}

#[derive(Debug, Default)]
pub struct SyncOutput {
    pub messages: Vec<IncomingMessage>,
    pub events: Vec<IncomingEvent>,
    pub documents: Vec<IncomingDocument>,
    /// Complete list of open tasks assigned to the user (a snapshot), plus recently closed ones.
    pub external_tasks: Option<Vec<IncomingExternalTask>>,
    /// True when the external task list was truncated (don't infer "closed" from absence).
    pub snapshot_incomplete: bool,
    /// (stream, new cursor) — persisted only after the batch is stored.
    pub cursors: Vec<(String, String)>,
    /// Stream names whose stored cursor must be dropped (full resync next time).
    pub reset: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    /// oauth (browser sign-in) | token (personal API token) | import (local files)
    pub support: &'static str,
    pub streams: &'static [&'static str],
    pub description: &'static str,
    pub permissions: &'static [&'static str],
    pub setup: &'static str,
}

pub fn catalog() -> Vec<ProviderInfo> {
    vec![
        ProviderInfo {
            id: "google",
            name: "Gmail + Google Calendar",
            category: "email",
            support: "oauth",
            streams: &["mail", "calendar"],
            description: "Reads recent Gmail messages (inbox and sent) and your primary Google Calendar. Read-only.",
            permissions: &["gmail.readonly — read messages and their metadata", "calendar.readonly — read events", "openid email — identify which account you connected"],
            setup: "Uses your Google Cloud OAuth client (Desktop app). See docs/SETUP.md §4.",
        },
        ProviderInfo {
            id: "microsoft",
            name: "Microsoft 365 / Outlook (Mail + Calendar)",
            category: "email",
            support: "oauth",
            streams: &["mail", "calendar"],
            description: "Reads Outlook / Microsoft 365 inbox and sent items and your calendar through Microsoft Graph. Read-only.",
            permissions: &["Mail.Read — read your mail", "Calendars.Read — read your calendars", "User.Read — read your profile (email address)", "offline_access — stay signed in"],
            setup: "Uses your Microsoft Entra app registration. See docs/SETUP.md §5.",
        },
        ProviderInfo {
            id: "slack",
            name: "Slack",
            category: "chat",
            support: "oauth",
            streams: &["chat"],
            description: "Direct messages, group DMs, channel messages that @mention you, and your own channel posts — so requests to you and promises you make in Slack become tasks. Read-only.",
            permissions: &["im:history, mpim:history — your direct and group messages", "channels:history, groups:history, search:read — channel messages that mention you or that you wrote", "users:read, users:read.email — names of people in messages"],
            setup: "Uses your Slack app (Client ID only, PKCE enabled). See docs/SETUP.md §6.",
        },
        ProviderInfo {
            id: "teams",
            name: "Microsoft Teams",
            category: "chat",
            support: "oauth",
            streams: &["chat"],
            description: "Your 1:1 and group chats in Teams. Read-only. (Channel conversations need an admin-approved permission and are not read.)",
            permissions: &["Chat.Read — read your chats (no admin approval needed)", "User.Read, offline_access"],
            setup: "Uses the same Microsoft app registration — add the Chat.Read permission. See docs/SETUP.md §7.",
        },
        ProviderInfo {
            id: "jira",
            name: "Jira (Cloud)",
            category: "tasks",
            support: "token",
            streams: &["tasks"],
            description: "Issues assigned to you. They appear in My tasks, merge with matching commitments from email and chat, and close in Kept when they're done in Jira.",
            permissions: &["Your Jira API token (acts as you, read only by Kept)"],
            setup: "Create an API token at id.atlassian.com → Security → API tokens. See docs/SETUP.md §8.",
        },
        ProviderInfo {
            id: "asana",
            name: "Asana",
            category: "tasks",
            support: "token",
            streams: &["tasks"],
            description: "Tasks assigned to you in all your workspaces, merged with detected commitments and closed when completed in Asana.",
            permissions: &["Your Asana personal access token (read only by Kept)"],
            setup: "Create a token in Asana → Settings → Apps → Developer apps → Personal access token. See docs/SETUP.md §8.",
        },
        ProviderInfo {
            id: "todoist",
            name: "Todoist",
            category: "tasks",
            support: "token",
            streams: &["tasks"],
            description: "Your open Todoist tasks, merged with detected commitments. You can also send a Kept task to Todoist — only after confirming.",
            permissions: &["Your Todoist API token (read; creates a task only when you confirm “Send to Todoist”)"],
            setup: "Copy your API token from Todoist → Settings → Integrations → Developer. See docs/SETUP.md §8.",
        },
        ProviderInfo {
            id: "linear",
            name: "Linear",
            category: "tasks",
            support: "token",
            streams: &["tasks"],
            description: "Issues assigned to you, merged with detected commitments and closed when completed in Linear.",
            permissions: &["Your Linear personal API key (read only by Kept)"],
            setup: "Create a key in Linear → Settings → Security & access → Personal API keys. See docs/SETUP.md §8.",
        },
        ProviderInfo {
            id: "gdrive",
            name: "Google Drive",
            category: "documents",
            support: "oauth",
            streams: &["documents"],
            description: "Names and links of files you edited or that were shared with you in the last 60 days, shown next to related tasks and meetings. File contents are never read.",
            permissions: &["drive.metadata.readonly — file names, links and dates only"],
            setup: "Uses your Google OAuth client — enable the Google Drive API. See docs/SETUP.md §4.",
        },
        ProviderInfo {
            id: "onedrive",
            name: "OneDrive & SharePoint",
            category: "documents",
            support: "oauth",
            streams: &["documents"],
            description: "Names and links of recent OneDrive and SharePoint files you can access, shown next to related tasks and meetings. File contents are never read.",
            permissions: &["Files.Read.All — find files you can access (Kept reads names, links and dates only)", "User.Read, offline_access"],
            setup: "Uses the same Microsoft app registration — add Files.Read.All. See docs/SETUP.md §7.",
        },
        ProviderInfo {
            id: "ics_file",
            name: "Calendar file (.ics)",
            category: "files",
            support: "import",
            streams: &["calendar"],
            description: "Import events from an iCalendar file exported from any calendar app.",
            permissions: &["Reads only the file you choose"],
            setup: "No account needed.",
        },
        ProviderInfo {
            id: "eml_file",
            name: "Email files (.eml / .json)",
            category: "files",
            support: "import",
            streams: &["mail"],
            description: "Import saved emails (.eml) or a Kept JSON export.",
            permissions: &["Reads only the files you choose"],
            setup: "No account needed.",
        },
    ]
}
