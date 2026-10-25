# Agent installation and recovery

Use this guide when an agent needs Android Use installed and connected to one device. The release `au` carries the Android helper, so nothing needs to sit beside it; `au install` writes agent configuration for you.

## Copy-paste setup prompt

```text
Set up Android Use for this agent and connect one Android device. Inspect the existing installation and preserve unrelated files, credentials, enrollments, and agent settings. Use the official release or repository source, verify checksums when downloading, and run the matching au setup command once. Keep the setup local; do not use raw ADB or bypass Android prompts. If the device is not authorized, tell me to unlock it, enable Developer options and USB debugging, reconnect it, and accept the USB debugging prompt, then wait. When Android asks for accessibility, tell me to open Settings, Accessibility, Android-Use, turn it on, and approve the warning; then resume with au doctor. Register the local stdio MCP server (serve --mcp) with each agent using au install, or au setup, which does it for every agent found (client-specific configs: docs/agents/clients.md). Verify with android_read command status and android_read command screen. Use the command-string tools for normal work, and never replay a partial or unknown mutation.
```

## Host install

One command: `npx -y android-use@1.4.0 setup` (npm), `brew install austinintelligence/tap/android-use` then `au setup`, or the install script (`install/install.sh`, `install/install.ps1`), which takes the latest release and verifies `SHA256SUMS` before extracting. Platforms: macOS, Linux, and Windows on x86_64 and arm64. From a verified release archive:

```powershell
.\au.exe doctor
.\au.exe setup
```

```sh
./au doctor
./au setup
```

If npm or Homebrew is unavailable, download the matching archive, `SHA256SUMS`, and `release-manifest.json` from the GitHub release and verify them before extraction. If Platform-Tools are installed elsewhere, set `AU_ADB` to the trusted executable.

## Device approvals

The device must be Android 8 or newer, unlocked, USB-debugging authorized, and the only enrolled hardware. `au setup` installs or updates the helper. Android still owns Accessibility, camera, microphone, notifications, location, and screen-recording approvals. Grant optional permissions only when the task needs them.

## Recovery

Run `au doctor` after every approval or connection change. Follow its `phase`, `next_step.kind`, ordered steps, and `resume` command. `agent` steps are host work; `user` steps require the Android device; `computer` steps repair a host dependency; `ready` means start the local MCP server. Use `au repair PATH` for a known helper APK and `au update` for a bundled update. Do not repeat an uncertain device mutation.

## Skill and MCP

The source skill is [`skills/android-use/SKILL.md`](../../skills/android-use/SKILL.md). Register that file with the agent's native skill installer, or keep it in the project and tell the agent to read it. `au install [AGENT]` registers `serve --mcp` and, for Claude Code and Codex, the skill; other MCP entries are preserved ([per-client configs](clients.md)). Reload the client. The server advertises only `android_read` and `android_act`, each with one required `command` string.
