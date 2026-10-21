# Integration support matrix

Every integration listed in the app is implemented. Each one was tested in two ways:

- **Mocked tests** (automated): the real sync code runs against a local fake server that returns the responses each provider documents.
- **Live tests:** connected to a real account.

| Integration | Sign-in | What Kept reads | Sync method | Mocked tests | Live-tested |
|---|---|---|---|---|---|
| **Gmail** | OAuth 2.0 + PKCE, loopback `127.0.0.1` (Google "Desktop app" client) | Inbox and sent mail (spam, trash, drafts, promotions and social skipped) | `messages.list`, then `history.list` from `historyId`; on expiry, a 3-day resync | ✅ | ✅ by the project owner |
| **Google Calendar** | Same Google sign-in | Primary calendar, −2 to +30 days, including cancellations | Window re-read each sync | ✅ | ✅ by the project owner |
| **Google Drive** | Separate Google consent, `drive.metadata.readonly` | Names, links and dates of files changed in the last 60 days (no contents) | `files.list` by modified time | ✅ | not yet |
| **Microsoft 365 / Outlook mail** | OAuth 2.0 + PKCE, public client, `http://localhost` | Inbox and Sent Items (drafts skipped) | Graph delta queries | ✅ | not yet |
| **Microsoft 365 Calendar** | Same Microsoft sign-in | `calendarView`, −2 to +30 days | Window re-read each sync | ✅ | not yet |
| **Microsoft Teams** | Microsoft sign-in, `Chat.Read` (no admin consent) | 1:1 and group chats | `/me/chats` by last message, then `/chats/{id}/messages` newer than the cursor | ✅ | not yet |
| **OneDrive & SharePoint** | Microsoft sign-in, `Files.Read.All` | Names, links and dates of files you can access, changed in the last 60 days (no contents) | Microsoft Search API (`/search/query`, driveItem) | ✅ | not yet |
| **Slack** | OAuth v2 + **PKCE** (Slack's desktop flow, no client secret), redirect `http://localhost:53682/callback` | DMs, group DMs, channel messages that @mention you, and your own channel posts | `conversations.history` and `search.messages` newer than the cursor | ✅ | not yet |
| **Jira Cloud** | Personal API token (Basic auth: email + token) | Issues assigned to you, open or resolved in the last 14 days | `POST /rest/api/3/search/jql` | ✅ | not yet |
| **Asana** | Personal access token | Tasks assigned to you in every workspace | `GET /tasks?assignee=me&completed_since=…` | ✅ | not yet |
| **Todoist** | Personal API token | Your open tasks. Can create a task, **only after you confirm** | API v1 `GET /api/v1/tasks`; `POST /api/v1/tasks` for Send to Todoist | ✅ | not yet |
| **Linear** | Personal API key | Issues assigned to you, open or completed in the last 14 days | GraphQL `viewer.assignedIssues` | ✅ | not yet |
| **Calendar file (.ics)** | — | VEVENTs, TZID, attendees, simple daily/weekly RRULE | Manual import | ✅ | ✅ (files) |
| **Email files (.eml / .json)** | — | RFC 5322 messages, with threads from References | Manual import | ✅ | ✅ (files) |

## Why some services use a personal token instead of browser sign-in

Jira, Asana, Todoist and Linear require a **client secret held on a server** in their browser sign-in flows. Asana's documentation, for example, says the secret "should never be exposed in client-side code". Kept has no server, by design: your data stays on your PC. For these tools Kept therefore uses each tool's official personal API token. The token is:

- created by you;
- shown to Kept once, then stored only in Windows Credential Manager;
- revocable by you at any time in the tool's settings.

Google, Microsoft and Slack all support desktop sign-in without a server-held secret (PKCE), so they use the browser.

## How task-tool items behave

| Situation | What Kept does |
|---|---|
| A new item is assigned to you | Creates a task in My tasks with the tool's due date and priority, and a link back. |
| The same work was already detected from email or chat | Links the two instead of creating a duplicate. |
| The due date changes in the tool | Updates Kept's deadline, keeping the old one in the deadline history. |
| The item is completed in the tool | Completes the Kept task, with "Marked done in Jira (OPS-12)" as the evidence. |
| The item disappears (reassigned, deleted, moved) | Puts the task in **Review** and asks you. It does not assume the work is done. |
| You delete the mirrored task in Kept | It stays deleted; Kept doesn't re-create it. |
| You disconnect the tool | Optionally removes the copied tasks. |

## Known provider limits

- **Teams channels:** reading them needs `ChannelMessage.Read.All`, which always requires an administrator. Kept reads chats only.
- **Slack:**
  - Workspaces that restrict apps need an admin to approve the Kept app once.
  - Slack expires desktop (PKCE) sign-ins that aren't used for 30 days.
  - Kept reads up to the 40 most recently active DMs and group DMs per sync.
- **Google Drive** uses a restricted scope (`drive.metadata.readonly`), like Gmail, so public distribution needs Google verification.
- **Jira:** only Atlassian *Cloud* is supported. Jira Data Center and Server use different authentication.
- **Rate limits:** every connector honours `Retry-After` and backs off automatically. Large workspaces are read in bounded batches per sync.

## Live verification checklist

Do this before publishing, for each integration you'll offer:

1. Connect it. The account shows **Connected**, with a recent *Last successful sync*.
2. Send yourself a test message (chat) or assign yourself a test item (tool). Within one sync interval it appears in Review or My tasks, with a working **Open original** link.
3. Complete the item in the tool (task tools). Within one sync it shows as done in Kept, with evidence.
4. Disconnect. The synced data disappears from Kept.

Gmail and Google Calendar have been through this by the project owner. The other integrations have only passed the mocked tests and need this check.
