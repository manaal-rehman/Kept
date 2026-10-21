# Troubleshooting

**Where to look first:** `%LOCALAPPDATA%\com.kept.agent\logs\kept.YYYY-MM-DD.log`, also reachable from Settings → Diagnostics → *Open log folder*. Logs never contain message text, tokens or keys. For more detail, quit Kept, then start it from PowerShell with `$env:KEPT_LOG="debug"; & "$env:LOCALAPPDATA\Programs\Kept\kept.exe"`.

## Building

| Symptom | Fix |
|---|---|
| `link.exe not found` / `MSVC` errors | Install the Visual Studio Build Tools "Desktop development with C++" workload (scripts/setup-windows.ps1), then open a **new** terminal. |
| `cargo` or `npm` not recognised | Open a new PowerShell after installing. Check with `cargo --version` and `node --version`. |
| Scripts are blocked ("running scripts is disabled") | Run `Set-ExecutionPolicy -Scope Process Bypass` in that window first. |
| Build is very slow or uses huge disk | The first build compiles about 400 crates (5–15 minutes, ~3 GB in `target\`). Keep the project out of OneDrive. Later builds are fast. |
| `npm ci` fails | Delete `node_modules` and run `npm install`. |

## Running

| Symptom | Fix |
|---|---|
| "Windows protected your PC" | The installer is unsigned. Click *More info → Run anyway*, or sign it (SHARING.md). |
| The window doesn't appear on launch | Kept may be set to start minimised. Click the tray icon (it may be hidden under the ^ arrow). |
| Kept vanished when I closed it | It's in the tray. If *Keep running in the tray* is off (Settings → Background), closing quits it. |
| No desktop notifications | Check Windows Settings → System → Notifications (Kept on, Focus/Do-not-disturb off). In Kept, check quiet hours, the pause state, and notification rules. Use *Send a test*. Notifications from `npm run app:dev` may show as coming from PowerShell; installed builds show "Kept". |
| Too many / too few reminders | Notifications → Rules (lead time, repeat, desktop priority), and Settings → Working hours (quiet hours, hourly limit). |
| "Credential Manager isn't available" | Rare (locked-down PCs). Sign-ins then last only until you quit. Ask IT whether Credential Manager is restricted by policy. |
| Times look off by hours | Set Settings → Working hours → Timezone to the same zone as Windows. |

## Connecting accounts

| Symptom | Fix |
|---|---|
| "No Google OAuth client is configured" | Do SETUP.md §4, then paste the Client ID and secret in Integrations → Google → App setup. |
| Google: "Access blocked: app has not completed verification" | Add your address under OAuth consent screen → **Test users**. |
| Google: `redirect_uri_mismatch` | The OAuth client type must be **Desktop app**, not Web. |
| Google asks to reconnect about every 7 days | Expected while the app is in Testing mode. Publish it to remove this (SETUP.md §4). |
| Microsoft: "Need admin approval" | Your organisation requires admin consent. Ask IT to grant consent for the app's Mail.Read and Calendars.Read permissions. |
| Microsoft: `AADSTS50011` redirect mismatch | Add platform *Mobile and desktop applications* with redirect `http://localhost`. |
| Browser shows "You're connected" but Kept says it failed | The error is on the Integrations card. Usually a firewall blocked the API call, or the client ID belongs to the wrong tenant. |
| Account shows *Needs attention* | The sign-in expired or was revoked. Click **Reconnect**. |
| "Offline or provider unreachable; retrying in N min" | A network problem. Kept retries with backoff automatically. *Sync now* forces a retry. |

## Detection quality

| Symptom | Fix |
|---|---|
| Too many false tasks | Reject them with *Not a task*; the same message won't create them again. Turn on *Review every detected task*, or raise the confidence threshold (Settings → General). |
| Missed commitments | Add an AI provider (Settings → AI provider). Make sure all your addresses and nicknames are in Settings → General, because ownership depends on them. |
| My own promises show as someone else's | Your sending address isn't in Settings → General → *Your email addresses*. |
| Two tasks for the same thing | Open one, and use the *possible duplicate* banner or Review → *Merge with existing*. |

## Reporting a problem

Include the following:

- the Kept version (Settings → Diagnostics);
- your Windows version;
- what you did, and what you expected;
- the relevant log lines.

Logs contain no message content, but check them before sharing anyway.
