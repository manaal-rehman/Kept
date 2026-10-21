# Known limitations (v0.3.0)

## Verification status

- **Live-tested by the project owner:** Gmail and Google Calendar, on Windows.
- **Not yet live-tested:**
  - Microsoft 365 / Outlook
  - Teams
  - OneDrive & SharePoint
  - Google Drive
  - Slack
  - Jira, Asana, Todoist and Linear

  Each is implemented against the provider's current documentation and passes automated tests against a mock of its API. They had not been run against real accounts when this was written (they need your credentials). Run the checklist in INTEGRATIONS.md before announcing each one publicly.
- **AI extraction quality** has not been measured with a real model. Run the harness in TESTING.md with your key.

## Functional limits

- **Teams:** chats only. Channel conversations need an admin-approved permission.
- **Slack:**
  - up to 40 recently active DMs and group DMs per sync;
  - channel messages only where you're @mentioned or are the author;
  - workspace admins may need to approve the app.
- **Documents** (Drive, OneDrive, SharePoint): file names, links and dates only, for the last 60 days. Matching to tasks and meetings is by file name.
- **Task tools:** read-only, except **Send to Todoist**, which runs only after your confirmation. Jira is Cloud only.
- **No other external write actions.** Kept never sends email, posts messages or changes calendars.
- **Rules extractor (no AI):** English only. On held-out phrasing it scores 91% precision and 67% recall. It handles one sentence at a time.
- **Dedup** uses keywords, threads and people, not semantic embeddings. Paraphrases that share few words can produce two tasks. **Merge** fixes this.
- **Meeting briefings** use calendar fields, synced messages and document names. Kept cannot read recordings or transcripts.
- **.ics import:** only simple DAILY/WEEKLY recurrences are expanded.
- **Workload estimates** are heuristics.
- **Date pickers** use the Windows timezone, so keep it the same as the timezone in Settings.

## Operational limits

- **The PC must be on.** Kept can't watch while the PC is off, asleep, offline or blocked by Battery Saver. It catches up when it wakes.
- **Google apps in Testing mode:**
  - limited to 100 test users;
  - refresh tokens expire after about 7 days;
  - users see an "unverified app" screen.

  Gmail and Drive use restricted scopes. Public release needs Google verification and a CASA security assessment.
- **Microsoft tenants** may require admin consent.
- **Slack PKCE sign-ins** expire after 30 days without use.
- **Unsigned installer:** SmartScreen warns until you sign it (SHARING.md).
- **No auto-update yet.** Install over the old version; your data is kept.
- **Local database encryption** relies on BitLocker or device encryption (SECURITY.md).

## Suggested next steps

1. Live-check each connector (INTEGRATIONS.md checklist) and fix anything the real APIs reveal.
2. Measure AI extraction with your key, and tune the prompt on the hold-out misses.
3. Add code signing and the Tauri updater.
4. Add semantic dedup for weak matches.
5. Add Teams channel mentions for tenants whose admins approve `ChannelMessage.Read.All`.
