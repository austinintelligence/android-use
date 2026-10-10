# Agent quickstart

1. Run `au setup` (or `npx -y android-use setup`) with one unlocked, USB-debugging-authorized device and approve Settings → Accessibility → Android-Use. It registers every agent it finds and reads the screen once to prove it works. `au doctor` explains anything missing.
2. To register one agent yourself: `au install claude` (also `codex`, `cursor`, `gemini`, `vscode`, `claude-desktop`, `opencode`). Manual configs: [client setup](clients.md).
3. Try it:

```text
android_read: status
android_read: screen
android_act: tap "Settings"
```

Use `page ...` commands for Chrome content. Actions report the resulting screen, so read only when the state is unknown. After `partial` or `unknown`, read before changing anything; never replay an uncertain action. Installation and repair: [install](install.md). Grammar and results: [agent protocol](../reference/agent-protocol.md).
