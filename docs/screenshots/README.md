# Screenshots — Placeholder Map

> Drop your real screenshots into this directory with the exact filenames below
> and the README will pick them up automatically.

| # | Filename | Where it's used | What it should show |
|---|----------|----------------|---------------------|
| 1 | `01-hero.png` | Hero (top of README) | The full main window: sidebar with sessions, chat panel, workspace panel. Best screenshot you have — first impression. |
| 2 | `02-agent-manager.png` | Agent Management | Settings → Agents page: list of detected agents (Claude Code, Codex CLI, Gemini CLI) with green "Installed" / red "Not Installed" status badges, plus a "Install" button showing install progress. |
| 3 | `03-chat-streaming.png` | Unified Chat | A live chat session mid-stream: thinking process block (collapsed), tool call expanded, final Markdown answer with a code block highlighted. |
| 4 | `04-workspace.png` | Project Workspace | A project open with: file tree on the left, Monaco editor in the middle showing a source file, xterm.js terminal at the bottom. |
| 5 | `05-model-hub.png` | Unified Model Hub | Settings → Models page: a list of configured providers/models with a clear "Set as default" / per-agent assignment selector. |
| 6 | `06-local-models.png` | Local Model Launcher | The local model manager: a list of downloaded GGUF models (DeepSeek Coder, Qwen, Llama, etc.), each with status (downloaded / not downloaded) and a "Start Server" button. |
| 7 | `07-app-manager.png` | App Manager | The app manager view: a grid or list of user-configured web apps (e.g. "Docs", "Internal Dashboard"), each with name, URL, icon, and an "Open" button. |
| 8 | `08-session-dashboard.png` | Session Dashboard | The session dashboard / kanban board view: each session as a card showing status (idle / running / error), current agent, project, last activity, token usage. |
| 9 | `09-cost-tracking.png` | Cost Tracking | A dashboard with a chart of token usage / cost over time, broken down by agent and model. |
| 10 | `10-protocol-proxy.png` | Protocol Proxy | A diagram-style or settings screenshot showing: Claude Code → proxy → OpenAI model (or any cross-protocol combo). Best to show the proxy in action. |

## How to add a screenshot

1. Take the screenshot (or export the image).
2. Save it with the exact filename above (e.g. `01-hero.png`).
3. Drop it into this `docs/screenshots/` directory.
4. Commit. The README will display it automatically.

## Image tips

- **Width**: 1200–1600 px for hero, 800–1200 px for the rest.
- **Format**: PNG for crisp UI; JPG is fine for hero.
- **Theme**: Prefer a clean light or dark theme — be consistent across all 10.
- **Crop**: Trim the OS window chrome; just show the app.
