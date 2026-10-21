# Kept architecture

## Why this stack

| Choice | What it is | Why it fits this app |
|---|---|---|
| **Tauri 2** | A framework for desktop apps that uses the operating system's built-in browser engine (WebView2 on Windows) for the window and Rust for everything else | Installers of about 5–10 MB, and roughly 20–40 MB of RAM for the background agent. Electron bundles a whole Chromium (100+ MB installer, 150+ MB RAM). Officially supports Windows 10 (1803+) and 11. |
| **Rust** | A compiled systems language | The agent runs all day, so it must be small, fast and crash-resistant. One process, no runtime to install. |
| **React + TypeScript + Vite** | The UI library, a typed JavaScript, and the bundler | Mature and fast to iterate. The whole UI ships as about 75 KB gzipped inside the binary. |
| **SQLite** (bundled through `rusqlite`) | An embedded database file | Zero setup, transactional, crash-safe with WAL journaling, and handles years of data in a few MB. |
| **Windows Credential Manager** (`keyring` crate) | The OS vault for secrets | Tokens and API keys never touch files, logs or the UI. |
| **OAuth 2.0 + PKCE, loopback redirect** (RFC 6749, 7636, 8252) | The standard sign-in flow for desktop apps | You sign in on Google's or Microsoft's own page. Kept never sees your password. |
| **Gmail API, Google Calendar API v3, Microsoft Graph v1.0** | Official provider APIs | Supported, documented and incremental, with no scraping. |
| **Anthropic Messages API / OpenAI Chat Completions** | Hosted LLM APIs, plus any OpenAI-compatible server | Swappable. A local model (Ollama) also works. |

**Why not Python?** A Python background service would need its own runtime, a packaging step (PyInstaller) and a second process talking to the UI. It adds 30–80 MB and gives no advantage here, because nothing in Kept needs Python-only libraries.

## Process model

```
┌──────────────────────────── kept.exe (one process) ────────────────────────────┐
│                                                                                 │
│  Tray icon ── menu: Open · Sync now · Pause · Silence 1h · Quit                 │
│                                                                                 │
│  Agent loop (Tokio task, every 60 s or on "Sync now")                           │
│    1 detect sleep/resume gap ──► resync + recompute reminders                   │
│    2 sync due accounts  ──► connectors (Gmail / Graph)  ──► store (dedup)       │
│    3 extract pending items ──► AI (validated) or rules  ──► tasks / evidence    │
│    4 reprioritise, refresh meeting prep                                         │
│    5 notification engine ──► Windows toasts (+ in-app centre)                   │
│                                                                                 │
│  Dashboard window (WebView2) ── created on demand, DESTROYED on close           │
│       │  IPC (typed commands, JSON)                                             │
│       ▼                                                                         │
│  kept-core (library, no UI code)                                                │
│    db · prefs · connectors · ingest · extract · ai · dates · tasks              │
│    meetings · planner · notify · followups · agent                              │
│                                                                                 │
│  %LOCALAPPDATA%\com.kept.agent\kept.db  (SQLite, WAL)                           │
│  Windows Credential Manager: com.kept.agent/oauth:<account>, ai:<provider>      │
└─────────────────────────────────────────────────────────────────────────────────┘
```

Closing the window destroys the webview, which frees most of the memory. The `RunEvent::ExitRequested` handler keeps the process alive, so the agent continues in the tray. **Quit Kept** in the tray ends it.

## Separation of concerns (code map)

| Concern | Where |
|---|---|
| Desktop shell, tray, window lifecycle, toasts, autostart | `src-tauri/src/main.rs` |
| IPC commands (input validation, write-only secrets) | `src-tauri/src/commands.rs` |
| OS credential store with chunking for long tokens | `src-tauri/src/keychain.rs` |
| Background scheduler and orchestration | `crates/kept-core/src/agent.rs` (`tick`) |
| Connectors (provider-agnostic model) | `crates/kept-core/src/connectors/` |
| AI reasoning layer | `crates/kept-core/src/ai.rs`, `extract/ai.rs` |
| Deterministic extraction fallback | `crates/kept-core/src/extract/rules.rs` |
| Deadline arithmetic (never done by AI) | `crates/kept-core/src/dates.rs` |
| Task lifecycle, evidence, dedup, merge, undo | `crates/kept-core/src/tasks.rs`, `ingest.rs` |
| Notification policy | `crates/kept-core/src/notify.rs` |
| Database schema and migrations | `crates/kept-core/migrations/`, `db.rs` |
| UI | `ui/src/` (views per screen; `api.ts` holds the typed IPC) |

## AI versus deterministic code

| AI does | Deterministic code does |
|---|---|
| Interpret meaning: is this a promise, a request or a hypothetical, and who owns it? | Compute every date from the quoted phrase, relative to send time and your timezone |
| Write a short task title from the user's point of view | Check every AI item quotes text that **really exists** in the message, and drop it otherwise |
| Link a message to an existing task (from a short list of candidates) | Dedup by thread, people and keyword similarity, plus deadline precedence |
| Optional 3–5 sentence meeting summary from listed facts | Priorities, daily plan, notification timing, quiet hours, rate limits |
| | Permissions: AI has no tools and cannot act. Its output is only suggestions. |

The AI output schema is defined in `extract/ai.rs` (`SYSTEM_PROMPT`) and enforced by `validate()`:

- unknown kinds, ungrounded quotes, misattributed promises (an "I'll do it" in a message the user didn't write) and out-of-range values are rejected;
- malformed JSON gets one repair retry, then falls back to rules;
- rate limits, timeouts and network errors back off AI and fall back to rules for that message.

## Data flow for one email

1. **Sync.** The connector produces `IncomingMessage`s. `store_messages` inserts each one keyed by `(account, external_id)`, with a content hash. The same message seen again is skipped. An edited message is re-processed.
2. **Prepare.** For each pending item, Kept collects up to 4 earlier messages in the thread and the open tasks that look related (keyword overlap, same thread).
3. **Extract.** The AI or rules extractor returns `Extracted` items. Possible kinds: `my_commitment`, `request_to_me`, `approval_request`, `implicit_assignment`, `reply_needed`, `followup_nudge`, `their_commitment`, `completion_evidence` or `approval_received`.
4. **Apply** (`ingest::apply`), in one transaction:
   - Task-creating kinds go through dedup. A **strong** match adds a source and applies deadline precedence. A **weak** match creates the task flagged *possible duplicate*.
   - Low confidence, ambiguity, injection suspicion or a non-owner role sends the item to **Review**. Nothing in Review triggers reminders.
   - `their_commitment` becomes a waiting-for record. It never becomes your task.
   - `completion_evidence` with stage `sent` and a strong match completes the task, with the evidence recorded. If the completion criteria need approval, or the stage is `draft_sent`, the task becomes **Awaiting approval** instead, never completed.
   - `approval_received` in the same thread completes the awaiting task, with evidence.
   - `followup_nudge` finds the original promise: it raises the priority and adds the source. If that task had been marked done, it reopens it for confirmation.
5. **Plan and notify.** Priorities and meeting prep states are recomputed, and the notification engine produces stage-based alerts.

## Deadlines

The `task_deadlines` table keeps the full history: `explicit` (stated: "by Friday"), `inferred` (implied: "I'll send it tomorrow"), `suggested` (never applied automatically) and `confirmed` (set by you). Precedence is confirmed > explicit > inferred, and a suggestion is never applied without you. Conflicts between two explicit dates from different people, or any change to a date you confirmed, are **flagged, not applied**. Meeting-relative deadlines ("before our Monday meeting") resolve against your calendar to *meeting start minus your prep buffer*.

## Synchronisation and resilience

| Situation | Behaviour |
|---|---|
| Incremental sync | Gmail uses `history.list` from the stored `historyId`. Outlook uses Graph delta links for Inbox and Sent Items. Calendars re-read a bounded window of −2 to +30 days each run, which also catches edits and cancellations. |
| Expired cursor | Gmail history expires after about a week: Kept re-syncs the last 3 days, and dedup prevents duplicates. Graph returns 410: the delta is reset. |
| Rate limiting | `Retry-After` is honoured, and the account stays *connected* with a visible "retrying in…". |
| Transient and offline errors | Exponential backoff from 2 to 60 minutes, shown as "Offline or provider unreachable". |
| Token revoked or expired | The account moves to **Needs attention → Reconnect**, and Kept stops retrying until you act. |
| Sleep, hibernate, offline | The loop notices the time gap, syncs everything immediately, and only the *current* reminder stage is generated, so you don't get a burst of stale alerts. |
| Crash or restart | All state is in SQLite (WAL). Pending notifications and cursors persist. Processed items are never re-processed. |
| Concurrency | A Tokio mutex serialises ticks. The DB mutex is never held across network calls. |

## Database

Schema: `crates/kept-core/migrations/0001_initial.sql`. The main entities:

- accounts, sync cursors and source items (bounded bodies)
- meetings and attendees, projects
- tasks with subtasks (`parent_id`), task dependencies, task sources (evidence links) and task deadlines
- commitments (including other people's promises)
- notifications and notification rules
- AI runs (metadata only: never content)
- preferences, audit events, and external actions (reserved for future write-actions; nothing writes externally today)

Migrations are versioned in `schema_migrations` and applied in a transaction on start. There are indexes on status, due date, thread, meeting and entity lookups. `PRAGMA secure_delete` overwrites deleted content. Online backup uses SQLite's backup API (Settings → Back up database).

## Extending: adding a provider

1. Add `connectors/<provider>.rs` that maps the API into `IncomingMessage` / `IncomingEvent`, using `connectors::http::get_json` for consistent error mapping.
2. Add its OAuth endpoints, if it uses OAuth, via `oauth::ProviderEndpoints`.
3. Add a `ProviderInfo` entry to `connectors::catalog()`, and a branch in `Agent::fetch` / `Agent::connect`.
4. Add mocked tests like `tests/mocked.rs`.

Nothing else changes: extraction, dedup, notifications and the UI work on the common model.
