# Setting up Kept on Windows

This guide covers four things:

1. Where to put the project files.
2. What to install.
3. How to build and run Kept.
4. How to connect Gmail, Microsoft 365 and an AI provider.

Every step happens on **your Windows 10 or 11 PC**. Nothing needs to run in the cloud, and Claude does not need to stay open.

> **Two ways to get an installer:**
> - **A. Build on your PC.** About 30–45 minutes the first time, mostly downloads. See §1–3.
> - **B. Let GitHub build it for you.** Free, and nothing to install locally. See §3b.
>
> Either way you end up with `Kept_0.3.0_x64-setup.exe`. Anyone can install that file without any developer tools.

---

## 1. Where to keep the project

Unzip `kept.zip` into a short path **without spaces**, for example:

```
C:\dev\kept
```

Avoid `Downloads`, OneDrive-synced folders and very long paths. Rust build folders contain thousands of files, and OneDrive syncing them slows everything down. The folder should contain `package.json`, `Cargo.toml`, `src-tauri\`, `crates\`, `ui\` and `docs\`.

## 2. Install the tools (once)

Open **PowerShell** (Start → type "PowerShell"), go to the folder and run the setup script:

```powershell
cd C:\dev\kept
Set-ExecutionPolicy -Scope Process Bypass   # lets this window run the bundled scripts
.\scripts\setup-windows.ps1
```

The script uses `winget` (built into Windows) to install the following:

| Tool | Why | Size |
|---|---|---|
| Microsoft C++ Build Tools | The linker Rust uses on Windows | ~2–4 GB |
| Rust (rustup) | Compiles the agent and the desktop shell | ~1 GB |
| Node.js LTS | Builds the user interface, and runs the Tauri command-line tool | ~100 MB |
| WebView2 Runtime | The browser engine that draws the window (already on Windows 11) | ~150 MB |

If you prefer to install by hand, you need the following:

- Visual Studio Build Tools 2022 with the "Desktop development with C++" workload
- Rust from https://rustup.rs
- Node.js 20 or newer from https://nodejs.org

**Close PowerShell and open a new one afterwards**, so the new tools are on your PATH.

## 3. Build and run

### Try it in development mode (hot reload)

```powershell
cd C:\dev\kept
npm install
npm run app:dev
```

The first run compiles everything, which takes 5–15 minutes. Later runs take seconds. The window opens with a short setup wizard. Choose **Explore with sample data** to see Kept work before you connect anything.

### Build the installer

```powershell
.\scripts\build-windows.ps1
```

This runs the automated tests and then builds two installers:

- `target\release\bundle\nsis\Kept_0.3.0_x64-setup.exe`: the recommended one. It installs per user and needs no admin rights.
- `target\release\bundle\msi\Kept_0.3.0_x64_en-US.msi`: for IT departments and Group Policy.

Double-click the `.exe` to install. Kept then appears in the Start menu. It runs in the system tray, keeps working when you close its window, and can start when you sign in (Settings → Background and startup).

### 3b. Build in the cloud with GitHub Actions (no local tools)

1. Create a free GitHub account and a **private** repository, then upload the project folder to it. You can use GitHub Desktop or `git push`.
2. In the repository, open **Actions → Build Windows installer → Run workflow**.
3. After about 15 minutes, open the finished run and download **kept-windows-installers** from *Artifacts*.

The workflow (`.github/workflows/build-windows.yml`) runs the tests on a real Windows machine before building.

---

## 4. Connect Gmail and Google Calendar (optional)

Google requires every app that reads Gmail to have its own OAuth client, so you register Kept once in your own Google Cloud project. It takes about 10 minutes and is free.

1. Go to https://console.cloud.google.com and create a project, for example "Kept".
2. Open **APIs & Services → Library** and enable **Gmail API**, **Google Calendar API** and, if you want document links, **Google Drive API**.
3. Open **APIs & Services → OAuth consent screen** (also called "Google Auth Platform"):
   - User type: **External**. Choose Internal if you use Google Workspace and only colleagues will use it.
   - Fill in the app name and your email.
   - Scopes: add `.../auth/gmail.readonly`, `.../auth/calendar.readonly`, `.../auth/drive.metadata.readonly`, `openid` and `email`.
   - **Test users: add your own Gmail address**, plus anyone you share Kept with.
4. Open **APIs & Services → Credentials → Create credentials → OAuth client ID**:
   - Application type: **Desktop app**. You don't need to enter a redirect URL, because desktop clients allow `http://127.0.0.1` on any port, which is what Kept uses.
   - Copy the **Client ID** and **Client secret**.
5. In Kept: **Integrations → Google → App setup**. Paste both values and click Save. The secret is stored in Windows Credential Manager, never in a file.
6. Click **Connect** on *Gmail + Google Calendar* and/or *Google Drive*; each is a separate, read-only consent. Your browser opens Google's own sign-in page. Approve the read-only permissions, and the tab says "You're connected".

**About "unverified app" warnings:** `gmail.readonly` is a *restricted* scope. While your app is in **Testing** mode, Google shows an "unverified app" screen. Click *Continue*, which is fine for yourself and the test users you listed, up to 100 people. Google also expires refresh tokens for apps in Testing mode after about 7 days. When that happens Kept shows **Needs attention → Reconnect**. To remove both limits for a wider rollout, publish the app and complete Google's verification (and, for restricted scopes, a security assessment).

## 5. Connect Microsoft 365 / Outlook (optional)

1. Go to https://entra.microsoft.com, then **Applications → App registrations → New registration**.
   - Name: Kept.
   - Supported account types: choose who may sign in. "Accounts in any organizational directory and personal Microsoft accounts" works for Outlook.com plus work accounts.
   - Redirect URI: platform **Public client/native (mobile & desktop)**, value `http://localhost`.
2. In the new app, open **Authentication** and set **Allow public client flows: Yes**. Kept uses PKCE and no client secret.
3. Open **API permissions → Add a permission → Microsoft Graph → Delegated** and add `Mail.Read`, `Calendars.Read`, `User.Read` and `offline_access`. If you'll use Teams or OneDrive/SharePoint, also add `Chat.Read` and `Files.Read.All` (see §7).
   - Some organisations require an admin to grant consent. If sign-in says "Need admin approval", ask IT to grant it for this app.
4. Copy the **Application (client) ID**.
5. In Kept: **Integrations → Microsoft → App setup**. Paste the ID, set the tenant (`common` is fine for most people) and Save. Then click **Connect**.

## 6. Connect Slack (optional)

Slack requires every app that reads messages to be a Slack app in your workspace. Kept uses Slack's desktop sign-in with PKCE, so no secret is needed.

1. Go to https://api.slack.com/apps → **Create New App → From an app manifest** → pick your workspace → paste:

   ```yaml
   display_information:
     name: Kept
     description: Personal assistant that tracks your commitments (read-only)
   oauth_config:
     redirect_urls:
       - http://localhost:53682/callback
     scopes:
       user:
         - im:history
         - im:read
         - mpim:history
         - mpim:read
         - channels:history
         - groups:history
         - users:read
         - users:read.email
         - search:read
     pkce_enabled: true
     token_rotation_enabled: true
   settings:
     org_deploy_enabled: false
     socket_mode_enabled: false
   ```

   If the manifest editor rejects `pkce_enabled`, create the app without it, then open **OAuth & Permissions** and turn on **PKCE** there. This is one-way: it marks the app as a public desktop client.
2. Copy the **Client ID** from **Basic Information → App Credentials**. You don't need the client secret.
3. In Kept: **Integrations → Slack → App setup**, paste the Client ID, then click **Connect**.
4. Some workspaces only let admins approve apps. In that case, Slack shows "Request to install". An admin approves it once.

What Kept reads:

- your DMs and group DMs;
- channel messages that @mention you;
- your own channel posts, because "I'll do it" in a channel is a commitment too.

It never posts. Sign-ins are kept fresh automatically. If Kept is off for more than 30 days, Slack expires the sign-in and you click **Reconnect**. Kept uses local port **53682** for the sign-in. If another program uses that port, Kept says so.

## 7. Connect Microsoft Teams and OneDrive / SharePoint (optional)

These use the **same** Microsoft app registration as §5.

1. In the app registration, open **API permissions → Add → Microsoft Graph → Delegated** and add:
   - `Chat.Read`, for Teams 1:1 and group chats. It needs no admin approval.
   - `Files.Read.All`, for OneDrive and SharePoint file names and links. Users can consent to it themselves in most tenants.
2. In Kept: **Integrations → Microsoft Teams → Connect**, and/or **OneDrive & SharePoint → Connect**.

**Teams channels:** reading channel conversations needs `ChannelMessage.Read.All`, which always requires an administrator. Kept reads chats only. In chats, a request in a 1:1 chat is treated as yours, while in a group chat it's treated as yours only if it names you.

**Documents:** Kept reads only file names, links and modification dates for files changed in the last 60 days, never file contents. It uses them to show *Related documents* next to tasks and meetings. Google Drive works the same way (§4).

## 8. Connect Jira, Asana, Todoist or Linear (optional)

For these tools Kept uses each tool's official **personal API token**. Their browser sign-in requires a client secret held on a server, which a desktop app can't keep secret, so a token you create, and can revoke, is the secure option.

| Tool | Where to create the token | What Kept also needs |
|---|---|---|
| Jira Cloud | https://id.atlassian.com/manage-profile/security/api-tokens → Create API token | Your site (`your-company.atlassian.net`) and Atlassian email |
| Asana | Asana → Settings → Apps → Developer apps → Create new token | — |
| Todoist | Todoist → Settings → Integrations → Developer → API token | — |
| Linear | Linear → Settings → Security & access → Personal API keys | — |

In Kept: **Integrations → (tool) → Connect**, then paste the token. Kept checks it immediately and stores it only in Windows Credential Manager.

What happens next:

- **Tasks assigned to you appear in My tasks.** Jira also includes issues resolved in the last 14 days, so closures are noticed. They keep the tool's due date and priority, and link back to the original.
- **Duplicates merge.** If Kept already found the same commitment in an email or chat, it links the two rather than creating a second task.
- **Completion is tracked.** When an item is completed in the tool, Kept completes it too, recording "Marked done in Jira" as the evidence. If an item simply disappears (reassigned or deleted), Kept asks you rather than assuming it's done.
- **Todoist only:** a task's details have **Send to Todoist**. It shows exactly what will be created and does nothing until you confirm.

## 9. Add an AI provider (optional, recommended)

Without AI, Kept uses built-in rules. They work offline and rarely invent tasks, but they miss more unusual phrasing (see `docs/TESTING.md` for measured numbers). To use AI:

1. Get an API key:
   - **Anthropic (Claude):** https://console.anthropic.com → API keys. API use is billed separately from any Claude.ai Pro/Max subscription. A chat subscription **does not** include API access.
   - **OpenAI:** https://platform.openai.com → API keys. This is likewise separate from ChatGPT Plus.
   - **Local and free:** install Ollama (https://ollama.com), run `ollama pull llama3.1`, choose "OpenAI-compatible" and set the base URL to `http://localhost:11434/v1`. Nothing leaves your PC, but you need a reasonably fast machine.
2. In Kept, go to **Settings → AI provider**. Choose the provider, paste the key, click **Save key**, then **Test connection**.
3. Optionally limit what AI may read under **Settings → Privacy and data**.

**Typical cost:** each new message is one small request of about 1–3k characters. A busy inbox of 100 messages a day costs roughly a few cents to $1–2 a month with a small model such as Claude Haiku, depending on current pricing. A daily request cap (default 300) prevents surprises.

## 10. Everyday use

- **Tray icon:** left-click opens Kept. Right-click gives Sync now, Pause monitoring, Silence notifications for 1 hour and Quit.
- **Closing the window** keeps the agent running. To stop Kept completely, use **Quit Kept** from the tray.
- **Start with Windows:** Settings → Background and startup.
- **Where your data is:** `%LOCALAPPDATA%\com.kept.agent\` holds the database and logs. Sign-ins and keys are in Windows Credential Manager under "com.kept.agent".
- **Keyboard:** `/` searches, `N` adds a new task, and `Alt+1`…`Alt+8` switch pages.

## 11. Baking app IDs into a build (for sharing)

If you share Kept with colleagues, you can build it with your Google, Microsoft and Slack app IDs included, so they only click **Connect**. Before building, set these in PowerShell:

```powershell
$env:KEPT_GOOGLE_CLIENT_ID = "1234-abc.apps.googleusercontent.com"
$env:KEPT_GOOGLE_CLIENT_SECRET = "GOCSPX-..."
$env:KEPT_MS_CLIENT_ID = "00000000-0000-0000-0000-000000000000"
$env:KEPT_SLACK_CLIENT_ID = "1234567890.1234567890"
.\scripts\build-windows.ps1
```

For GitHub Actions, add the same names as repository secrets.

For a Slack app used across *other* workspaces, enable **Manage Distribution** in the Slack app settings. Task-tool tokens are always personal, so each user creates their own. See `docs/SHARING.md` before giving the app to other people.
