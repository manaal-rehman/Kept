# Sharing Kept with other people

## The simple way

1. Build the installer (SETUP.md §3 or §3b). You get `Kept_0.3.0_x64-setup.exe`, about 5–10 MB.
2. Send that one file: a shared drive, Teams, email or a GitHub Release all work.
3. They double-click it. No admin rights are needed, because it installs per user under `%LOCALAPPDATA%\Programs\Kept`. If WebView2 is missing (rare on Windows 10, never on 11), the installer downloads it.
4. On first launch everyone gets their own private setup. Each person's data stays on their own PC.

People you share with **do not** need Rust, Node, Claude, or this project folder.

## How users connect: each person or organisation owns their connection

The public installer contains **no app IDs of yours**, so users never depend on you or your Google, Microsoft or Slack approvals.

- **Individuals:** Kept guides them through registering their own Google, Microsoft or Slack app the first time they click Connect. Google takes about 10 minutes, Microsoft about 5, and Slack about 3. Each person's Google app is used only by them, so Google's 100-user cap, its review, and the 7-day sign-out don't apply. Users see a one-time "unverified app" notice, which is expected for their own app. Details for users: [USER-GUIDE.md](USER-GUIDE.md).
- **Companies:** IT sets the connections up once (a Google Workspace *Internal* app, a Microsoft app with admin consent, and a Slack app) and shares a settings file. Staff click **Import settings file**, then **Connect**. Details: [ADMIN-GUIDE.md](ADMIN-GUIDE.md).
- **Jira, Asana, Todoist, Linear:** each person pastes their own personal token. Nothing to prepare.

**Optional, for a private build:** if you ever build Kept just for your own team, you can still bake app IDs in with the `KEPT_*_CLIENT_ID` variables (SETUP.md §11). Don't do this for the public installer.

Give users these two documents with the download: `USER-GUIDE.md` for everyone, and `ADMIN-GUIDE.md` for IT teams.

## AI for other users

Each person adds their own API key in Settings, or uses a local model (Ollama). Never build your key into the app: anyone could extract it.

## Code signing (removes the "Windows protected your PC" warning)

Unsigned installers trigger SmartScreen. To sign them:

1. Buy a code-signing certificate. An OV certificate costs about $200–400 a year; EV certificates get SmartScreen reputation immediately. Alternatively, use **Azure Trusted Signing**, Microsoft's signing service for about $10 a month.
2. Configure `bundle.windows.certificateThumbprint`, or a `signCommand` for Azure Trusted Signing, in `src-tauri/tauri.conf.json`. Tauri's guide: https://v2.tauri.app/distribute/sign/windows/
3. Rebuild. Both the `.exe` and `.msi` are signed.

## Publishing updates

- **Now:** bump `version` in `src-tauri/tauri.conf.json`, `package.json` and `Cargo.toml`, rebuild, and share the new installer. Installing over the old version keeps all data, and database migrations run automatically.
- **Later:** add `tauri-plugin-updater` with a signed `latest.json` hosted on GitHub Releases, so Kept updates itself. Tagging `v0.2.0` already makes the workflow attach installers to a GitHub Release.

## Backup and restore (for each user)

- **Back up:** Settings → Privacy and data → *Back up database* writes a `.db` snapshot. *Export my data* writes readable JSON.
- **Restore:** quit Kept (tray → Quit). Replace `%LOCALAPPDATA%\com.kept.agent\kept.db` with the backup, delete any `kept.db-wal` and `kept.db-shm` next to it, then start Kept. Sign-ins aren't in the backup, so reconnect accounts afterwards.

## Uninstall

Use Windows Settings → Apps → Kept → Uninstall. Uninstalling leaves your data in case you reinstall. To remove everything, first use Settings → *Delete all local data* (which also clears Credential Manager), or delete `%LOCALAPPDATA%\com.kept.agent` afterwards.
