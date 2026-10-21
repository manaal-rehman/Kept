# Kept for IT administrators

This guide covers setting Kept up once for your whole organisation. Your colleagues then import one file and sign in with their own accounts. No one depends on Kept's publisher, and Kept has no servers: each person's data stays on their own PC.

**What you'll produce:** `kept-connection-settings.json`, holding your organisation's app registrations for Google, Microsoft and Slack. It contains **no** sign-ins, tokens or user data.

## Google Workspace: an Internal app, with no Google review and no user limits

An **Internal** app can only be used by accounts in your Workspace organisation. Google doesn't require verification for it, there's no 100-user cap, and users aren't signed out every 7 days.

1. Sign in to https://console.cloud.google.com with an account in your organisation, and create a project named **Kept** under your organisation.
2. Enable the **Gmail API**, **Google Calendar API** and **Google Drive API**:
   https://console.cloud.google.com/flows/enableapi?apiid=gmail.googleapis.com,calendar-json.googleapis.com,drive.googleapis.com
3. Go to **Google Auth Platform → Branding → Get started**. App name: Kept. For **Audience**, choose **Internal**.
4. Go to **Clients → Create client**. Application type: **Desktop app**. Name: Kept. Copy the Client ID and secret.
5. **Trust the app in the Admin console.** This is required if your organisation restricts access to Gmail and Drive data, which Google calls high-risk scopes:
   1. Go to admin.google.com → **Security → Access and data control → API controls → Manage third-party app access**.
   2. Click **Configure new app**, then search for the Client ID.
   3. Choose who it applies to, and set it to **Trusted**.

**Trusting an existing app instead:** if someone has already registered a Kept app (for example, a colleague's own app set to External), you can mark that Client ID as **Trusted** in step 5. Your users can then use it without the per-user limits.

## Microsoft 365: one registration, approved once for everyone

1. Go to https://entra.microsoft.com → **App registrations → New registration**.
   - Name: Kept.
   - Supported accounts: **Accounts in this organizational directory only**.
   - Redirect URI: platform **Public client/native (mobile & desktop)**, value `http://localhost`.
2. Go to **API permissions → Add → Microsoft Graph → Delegated** and add the following permissions. All are read-only:

   | Permission | What it's for |
   |---|---|
   | `Mail.Read` | Outlook mail |
   | `Calendars.Read` | Calendar |
   | `Chat.Read` | Teams chats |
   | `Files.Read.All` | OneDrive and SharePoint file names and links |
   | `User.Read` | The signed-in user's profile |
   | `offline_access` | Staying signed in |
3. Click **Grant admin consent for (your organisation)**. Users then won't see consent prompts. You can grant consent later instead by visiting `https://login.microsoftonline.com/<tenant-id>/adminconsent?client_id=<client-id>`.
4. Copy the **Application (client) ID** and your **Directory (tenant) ID**.

**Notes:**
- Kept uses PKCE and no client secret, so there's nothing to rotate.
- Kept doesn't read Teams channel messages. That would need `ChannelMessage.Read.All`, which Kept doesn't request.

## Slack: one app for your workspace

1. Open the Slack app setup in Kept (**Integrations → Slack → Set up → Create the Slack app**). Alternatively, use the manifest in docs/SETUP.md §6 at https://api.slack.com/apps.
2. Create the app in your workspace. Make sure **PKCE** is on under *OAuth & Permissions*.
3. If your workspace requires app approval, approve **Kept** under Workspace settings → Manage apps.
4. Copy the **Client ID**.

**Enterprise Grid:** create the app at org level, or approve it per workspace.

## Create and share the settings file

1. In Kept on your own PC, enter the values: **Integrations → (service) → Set up → Paste it here**.
   - **Google:** Client ID and secret.
   - **Microsoft:** Client ID, plus your tenant ID or domain instead of `common`.
   - **Slack:** Client ID.
2. Click **Connect** once for each service to check it works.
3. Click **Share my setup with my team**. Enter your organisation's name, keep **Include the Google client secret** on, and save the file.
4. Share `kept-connection-settings.json` through your intranet, shared drive, or software deployment tool.

**About the Google secret:** Google treats desktop-app secrets as non-confidential, but share the file only inside your organisation anyway.

Your colleagues then follow these steps:

1. Install Kept.
2. Open **Integrations → Import settings file**.
3. Click **Connect** on each service and sign in with their own account.

That takes about 2 minutes, with no technical steps.

## Jira, Asana, Todoist, Linear

Each person connects these with their own personal API token, so you have nothing to set up. If your organisation restricts API tokens (for example, Atlassian Access policies), allow them for the people who'll use Kept.

## Security summary for your review

- **Read-only access** to mail, calendar, chats and document metadata. File contents are never read.
- **No Kept servers or telemetry.** Data is stored in each user's `%LOCALAPPDATA%\com.kept.agent`, and tokens in Windows Credential Manager.
- **Optional AI** uses a provider and API key each user chooses, and can be switched off or set to a local model. Credentials are always redacted before sending; contact details are redacted by default.
- **One write action:** Todoist task creation, only after the user confirms.
- **Uninstalling:** Kept leaves data in place until the user chooses *Delete all local data*, or removes `%LOCALAPPDATA%\com.kept.agent`.

Full details are in SECURITY.md.
