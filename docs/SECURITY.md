# Security and privacy

Kept reads sensitive work communication, so its security design starts from that fact. This document says plainly what Kept protects against and what it does not.

## What Kept accesses

| Data | Access | Where it goes |
|---|---|---|
| Gmail / Outlook messages (inbox and sent) | Read-only API scopes (`gmail.readonly`, `Mail.Read`) | A bounded copy (≤12,000 characters, quoted history removed when processing) in the local DB. The text is purged after N days (default 30), and the whole record after M days (default 180). |
| Calendar events | Read-only (`calendar.readonly`, `Calendars.Read`) | Local DB, for a window of −2 to +30 days |
| Your email address and name | `openid email` / `User.Read` | Local preferences, used to tell "you" from others |
| Slack / Teams messages | Read-only (Slack user scopes, `Chat.Read`) | DMs, group DMs and chats; Slack channel messages only where you're mentioned or the author. Same bounded storage and retention as email. |
| Drive / OneDrive / SharePoint | `drive.metadata.readonly`, `Files.Read.All` | **File names, links and dates only.** Contents are never downloaded. |
| Jira / Asana / Todoist / Linear | Your personal API token | Titles, status, due date, project and link of tasks assigned to you |
| Optional AI provider | Your API key | See "What is sent to AI" |

**Kept never sends, posts, replies, accepts, approves, deletes or edits anything in your accounts.** It requests no write scopes for email, chat, calendar or documents.

**The one write action is *Send to Todoist*:**

- It runs only when you click it in a task and then press **Create in Todoist** in a confirmation dialog. The dialog shows the destination, the content and the effect.
- The command also refuses to run without that confirmation flag.
- Every attempt is recorded in `external_actions` (proposed, then confirmed, then executed or failed).
- It refuses to push the same task twice.

**Personal API tokens** (Jira, Asana, Todoist, Linear) can do whatever your account can do in that tool. Kept's code only reads, apart from the confirmed Todoist action. Keep in mind:

- a token is as powerful as your account there;
- Kept stores it only in Credential Manager;
- disconnecting deletes it from your PC, and the UI shows where to revoke it in the tool.

These tools' browser sign-in needs a server-held client secret, which a desktop app cannot protect. A user-created, user-revocable token is the safer option without a Kept server.

## Secrets

- OAuth tokens, API keys and the Google client secret live only in **Windows Credential Manager** (service `com.kept.agent`), through the `keyring` crate. Long tokens are split across several entries because of the Windows 2,560-byte blob limit.
- The UI can **set** a secret but never **read** one back (`set_secret` / `has_secret`).
- Secrets are never written to SQLite, config files, logs or exports. A test checks that exports contain no tokens.
- Passwords are never requested. Sign-in happens on the provider's own page in your browser (OAuth 2.0 Authorization Code + PKCE, loopback redirect, `state` checked against CSRF).
- Disconnecting revokes the token where the provider allows it (Google, Slack), deletes stored tokens, and deletes all content synced from that account: messages, events, document names and mirrored tasks (optional). Microsoft has no self-revocation for public clients, so the UI tells you to remove consent at myapps.microsoft.com.

## Local data protection (threat model)

| Threat | Protection | Not protected |
|---|---|---|
| Other users on the same PC | The DB is in your `%LOCALAPPDATA%` (per-user ACLs). Credential Manager is per user. | Administrators can read any user's files. |
| Lost or stolen laptop | Rely on **BitLocker / Device Encryption** (recommended in-app). | The DB is **not separately encrypted**. Encrypting it (SQLCipher) would need a key, and that key would sit in the same Credential Manager. That adds complexity for little gain over BitLocker. |
| Malware running as you | — | Anything running as you can read your files and Credential Manager. No desktop app can defend against this. |
| Deleted data recovery | `PRAGMA secure_delete=ON` overwrites deleted rows. Retention purges old bodies. | Filesystem snapshots or backups you made yourself. |

Kept is **not end-to-end encrypted**, and does not claim to be.

## What is sent to the AI provider

Only when an AI provider is configured, and only for data types you allow (Settings → Privacy and data):

- the **newly written part** of one message (quoted history removed), cut to your limit (default 6,000 characters for the whole request);
- up to 4 short excerpts (≤600 characters each) of earlier messages in the same thread;
- the titles of up to 25 possibly related open tasks;
- your name and addresses, so the model can tell who "I" is.

Before anything is sent:

- Passwords, API keys, tokens (OpenAI, Slack, Google, JWT-like strings), long digit sequences (card numbers) and URL query strings are **always** redacted.
- Email addresses and phone numbers are redacted by default ("Hide email addresses…").
- Other people's addresses in To/Cc are replaced with placeholders when redaction is on.

The `ai_runs` table records only metadata: provider, model, purpose, character counts and errors. It never stores content. You can see it in Settings → Diagnostics. A daily request cap applies.

Kept never forwards whole mailboxes. A message is analysed **once**, when it first arrives (content hash plus processed state), and is not re-sent on every sync.

## Prompt injection

Emails are untrusted input and may contain text like "ignore your instructions and…". Kept defends against this in four layers:

1. **No capabilities to hijack.** The model has no tools, no network access and no write permissions. Its only output is JSON *suggestions*.
2. **Clear data boundaries.** The system prompt states that content inside `<message>`, `<thread>` and `<open_tasks>` is untrusted data and never instructions.
3. **Grounding check.** Every returned item must quote text that exists verbatim in the message, or it is discarded. Invented tasks such as "wire money", or attempts to mark tasks complete, are rejected. A promise attributed to the user in a message the user didn't write is rejected. Dates are recomputed deterministically.
4. **Detection.** Messages with injection-like phrases are flagged. Their items are forced into Review with a visible warning and confidence capped at 50%, for both the AI and rules extractors.

These layers are tested in `extract::ai::tests`, `tests/pipeline.rs::prompt_injection_is_contained` and `tests/mocked.rs`.

## Desktop shell hardening

- A strict Content-Security-Policy applies: only the app's own scripts run. There's no remote code, no `eval`, and no inline scripts.
- The webview gets only Kept's own commands plus notifications (`src-tauri/capabilities/default.json`). File dialogs, URL opening and autostart are performed in Rust after validation.
- `open_external` only opens `https:` and `mailto:` links.
- Message bodies are rendered as plain text, never as HTML.
- Logs (`%LOCALAPPDATA%\com.kept.agent\logs`, 7 days, rotated daily) record ids, counts and error kinds only, never message text or secrets.
- There is no telemetry, analytics or crash upload of any kind.

## Your controls

- **Settings → Privacy and data:** choose which data types AI may process, PII redaction, request size, retention periods, export (JSON), backup (SQLite), and *Delete all local data*. The last option also removes saved sign-ins and keys.
- **Integrations:** per-account mail/calendar toggles, and disconnect with data deletion.
- **Review queue:** accept, reject, merge, change owner. **History:** undo status, deadline, owner, priority and title changes.

## Known gaps (also in LIMITATIONS.md)

- No independent security audit has been performed.
- The installer is not code-signed until you add a certificate (see `docs/SHARING.md`). Windows SmartScreen will warn about unsigned builds.
- Google "Testing"-mode apps get refresh tokens that expire after about 7 days. Publishing the app requires Google verification.
