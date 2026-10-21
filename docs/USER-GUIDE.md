# Kept: getting started

Kept is a desktop app for Windows 10 and 11. It reads the work accounts you connect, finds the things you've promised or been asked to do, and reminds you before anything slips. Everything stays on your computer, and nothing is sent to the people who make Kept.

## 1. Install

1. Download `Kept_x.y.z_x64-setup.exe`.
2. Double-click it. If Windows shows "Windows protected your PC", click **More info → Run anyway**. This appears for apps that aren't yet widely downloaded.
3. Kept opens with a short setup: your name, your work email, and your working hours.

Kept lives in the **system tray**, the ^ arrow next to the clock. Closing the window keeps it reminding you. To stop Kept completely, use **Quit Kept** in the tray menu.

## 2. Connect your accounts

Open **Integrations** and click **Connect** next to what you use.

### Got a settings file from your IT team?
Click **Import settings file**, choose the file, then click **Connect** on each service. You'll sign in with your own account, so skip the rest of this section.

### Jira, Asana, Todoist, Linear: about 1 minute each
Kept asks for a personal token and shows you exactly where to create it in that tool. Paste it, click Connect, and you're done.

### Google (Gmail, Calendar, Drive): about 10 minutes, once
Google requires every app that reads Gmail to be registered, so you register Kept under **your own** Google account. Kept walks you through it step by step: each step has a button that opens the right page.

1. Create a Google Cloud project.
2. Turn on Gmail, Calendar and Drive.
3. Set up the sign-in screen.
4. Publish it for yourself.
5. Create the desktop sign-in key.
6. Paste the key into Kept.

When you connect, Google says **"Google hasn't verified this app."** That's expected, because it's your own app. Click **Advanced → Go to Kept**, then allow access.

### Microsoft (Outlook, Calendar, Teams, OneDrive): about 5 minutes, once
Kept guides you through registering it in your Microsoft account, much like Google. If your company doesn't let you register apps, ask IT for a settings file. Some companies also require IT to approve apps; Microsoft will tell you if so.

### Slack: about 3 minutes, once
Click **Set up** under Slack, then **Create the Slack app**. Slack opens with everything filled in. Choose your workspace, click Create, copy the Client ID into Kept, then click Connect. If your workspace requires approval, Slack asks an admin for you.

## 3. Everyday use

| Where | What you'll find |
|---|---|
| **Home** | What to do next, today's plan, overdue items and upcoming meetings |
| **Review** | Things Kept found but isn't sure about. Accept, edit, or click *Not a task*. Nothing here reminds you until you accept it. |
| **My tasks** | Everything you've committed to, from every connected source. Each task shows the exact sentence it came from. |
| **Meetings** | What to prepare, related messages and documents, and promises you made to attendees |
| **Follow-ups** | Replies you owe, people waiting on you, and things you're waiting for |

Keyboard shortcuts: `/` to search, `N` for a new task, and `Alt+1`…`Alt+8` to switch pages.

## 4. Optional: better detection with AI

Kept works without AI. Adding an AI key under **Settings → AI provider** makes it much better at spotting commitments phrased in unusual ways. You can use:

- **Anthropic (Claude):** a key from console.anthropic.com.
- **OpenAI:** a key from platform.openai.com.
- **A free local model:** Ollama.

An API key is billed separately from a Claude or ChatGPT subscription. Typical use costs a few cents to a couple of dollars a month.

## 5. Your data

- All your data stays on your computer. Sign-ins and keys are kept in Windows Credential Manager.
- Kept only reads. It never sends email, posts messages or changes your calendar. The one exception is **Send to Todoist**, which always asks you first.
- **Export, backup or delete:** go to Settings → Privacy and data.
- **Disconnect** any account in Integrations. Kept then deletes everything it synced from it.

## Need help?

See TROUBLESHOOTING.md, or Settings → Diagnostics → *Open log folder*. Logs never contain your messages.
