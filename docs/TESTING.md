# Testing

Run everything with:

```powershell
cargo test -p kept-core          # all Rust tests (about 10 s after the first build)
npm run typecheck                # UI type checks
```

## Three kinds of tests (kept separate on purpose)

| Kind | Files | Network | What it proves |
|---|---|---|---|
| **Automated unit and behaviour tests** | `src/**` `#[cfg(test)]`, `tests/pipeline.rs`, `tests/eval.rs` | none | Extraction, dates and timezones, dedup, lifecycle, notifications, persistence, export/delete, all on synthetic data |
| **Mocked integration tests** | `tests/mocked.rs`, `tests/connectors_mocked.rs` | local fake server only | Our HTTP, OAuth and sync-engine code paths behave correctly for documented provider responses and failures, and the AI pipeline validates, repairs and falls back |
| **Real-provider tests** | manual (see INTEGRATIONS.md checklist); optional `ai_extractor_quality` | real APIs | That each provider behaves as documented. **Done for Gmail and Google Calendar by the project owner on Windows; pending for the others.** |

## Results at delivery time

These results come from running `cargo test -p kept-core` in the development environment, on Linux x86-64 with Rust 1.97:

| Suite | Tests | Result |
|---|---|---|
| Unit tests (dates, text, OAuth/PKCE, all provider parsers, connection settings, AI validation) | 34 | pass |
| `tests/pipeline.rs` (end-to-end behaviour) | 22 | pass |
| `tests/mocked.rs` (HTTP, Gmail sync engine, AI pipeline) | 4 | pass |
| `tests/connectors_mocked.rs` (Slack, Teams, Drive, OneDrive/SharePoint, Jira, Asana, Todoist, Linear, end to end) | 1 (≈40 assertions) | pass |
| `tests/eval.rs` (extraction quality) | 2 (+1 optional, ignored) | pass |

The project owner built and ran the app on Windows 11 with Gmail and Google Calendar connected. `.github/workflows/build-windows.yml` repeats the tests on Windows for every build.

### Behaviour covered (`tests/pipeline.rs`)

- Explicit commitments ("I'll share the revised report tomorrow" gives an inferred date).
- Implicit ownership ("Since you're putting together the Q4 budget summary…") goes to review, without borrowing the other clause's date.
- Meeting-relative deadlines are due before the meeting minus the prep buffer.
- Negatives: CC-only, hypotheticals, other people's promises, past events, unrelated deadlines, completed exchanges.
- Email, revised email and chat reminder about the same thing merge into **one** task, and the deadline update stays traceable in its history.
- A follow-up ("you promised last week") finds the original promise and raises its priority.
- A draft sent becomes **Awaiting approval**, not complete. Receipts sent become **Completed** with evidence.
- "Sure, will do" becomes a task from the earlier request, with no invented deadline.
- Prompt injection: the item goes to review with a warning, and nothing else changes.
- Meetings without an agenda get prep state *unknown* and only labelled suggestions.
- Agents cannot complete tasks without evidence. Undo works.
- A deadline you confirmed is never silently overwritten; a conflict is flagged instead.
- Merge and reject.
- Re-syncing the same messages creates nothing new.
- Notifications are deduped, rate limited, snoozed and held in quiet hours, and closing a task expires its reminders.
- Sleep or late start produces one current-stage alert, not stale ones.
- State survives a restart: tasks, pending notifications, and no re-processing.
- Export contains no tokens. Disconnect deletes synced data. Delete-all leaves nothing.

### Every other connector (`tests/connectors_mocked.rs`)

- **Todoist:**
  - a bad token is rejected;
  - a mirrored task links to an email-detected task instead of duplicating it;
  - p1 maps to critical, and the due date to an explicit deadline;
  - completing in Todoist completes in Kept, with evidence;
  - a vanished task goes to review (not completed);
  - *Send to Todoist* creates exactly one task, records the action, and refuses a second push.
- **Jira:** Basic auth; JQL; resolved issues aren't imported; tasks link back to `/browse/KEY`.
- **Asana:** workspace discovery and assigned tasks with projects.
- **Linear:** raw API-key header; a bad key is rejected; assigned issues come through.
- **Slack:**
  - DMs (system messages skipped), channel @mentions and my own channel posts become the right tasks;
  - the cursor advances.
- **Teams:** chat members map to emails; the HTML body becomes text; an approval request becomes an approval task.
- **OneDrive/SharePoint** (Search API) and **Google Drive:** documents are indexed, and related-document matching works ("Q4 numbers" → *Q4 numbers summary.xlsx*) without false matches.
- **Disconnect** removes mirrored tasks and indexed documents.

### Mocked integration (`tests/mocked.rs`)

- HTTP mapping: 401 → reconnect needed, 429 → back off for Retry-After, 404/410 → cursor reset, 5xx → retried twice.
- Gmail engine: initial sync, then history-based incremental, then expired history (3-day resync with no duplicate tasks), then rate limited (stays connected, backs off), then revoked (needs attention).
- OAuth refresh with `invalid_grant` → reconnect needed.
- AI pipeline against a fake OpenAI-compatible server:
  - a grounded item is accepted, and its date is computed deterministically;
  - an ungrounded "wire money" item is rejected;
  - malformed output is repaired with one retry;
  - a rate limit falls back to rules;
  - `ai_runs` stores no content.

## Extraction quality (measured, synthetic data)

`sample-data/eval/` contains two labelled sets:

- `extraction_cases.json`: 53 cases (33 that should produce a task, 20 that should not; 3 deadline-statement cases added after a user report). The rules were tuned while looking at these, so the scores **overstate** real-world quality.
- `extraction_holdout.json`: 25 cases written afterwards and **never used for tuning**. This is the honest estimate.

| Built-in rules extractor | Precision | Recall | Notes |
|---|---|---|---|
| Dev set (tuned on) | 100% | 100% | Overfit by construction |
| **Hold-out set** | **91%** | **67%** | Misses indirect phrasing such as "We're blocked until you confirm…", "Just a nudge on…", "It'd be great if you could…", "you'll own…", and sentences without a subject ("Will send…"). One false positive: "will you be at the party friday?" |

What this means: without AI, Kept seldom invents tasks but misses about a third of unusual phrasings. That's why the AI provider is recommended.

To measure the AI extractor on the same sets with your own key (synthetic data only, a few cents):

```powershell
$env:KEPT_EVAL_PROVIDER="anthropic"; $env:KEPT_EVAL_API_KEY="sk-ant-..."
cargo test -p kept-core --test eval ai_extractor_quality -- --ignored --nocapture
```

No AI-extractor numbers are reported here, because that test was not run (it needs a key).

## Manual checks done in development

The real desktop app (debug build, Linux and WebKitGTK, under a virtual display) was launched and driven:

- onboarding, Explore with sample data, the home dashboard;
- Review, My tasks (list), the task drawer, Meetings and the briefing, Follow-ups, Notifications, Integrations and Settings;
- all of these rendered with live data from the Rust core over IPC.

The following were **not** exercised and need checking on Windows:

- Windows toasts;
- the tray icon (no tray host in the virtual display);
- autostart;
- Credential Manager;
- the NSIS/MSI installers;
- RAM and CPU on Windows.

## Resource usage

| Measure | Value | Notes |
|---|---|---|
| UI bundle | ~75 KB JS + 4.5 KB CSS gzipped | from `vite build` |
| Release binary | 13.3 MB on Linux x86-64 (includes GTK bindings) | Windows binaries are usually smaller. The NSIS installer compresses to an estimated 4–7 MB (not measured). |
| Background work | One tick per minute (a few SQL queries). Network calls only at the sync interval (default 10 min). AI only for new messages. | |
| Memory | Not measured on Windows. Tauri apps typically use 20–40 MB with the window closed (webview destroyed) and 80–150 MB with it open (WebView2). | Please measure in Task Manager |
