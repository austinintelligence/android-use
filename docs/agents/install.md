# Agent installation and recovery

Use this guide when an agent needs Android Use installed and connected to one device. Keep `au` and `aubridge.apk` together and use absolute paths in agent configuration.

## Copy-paste setup prompt

```text
Set up Android Use for this agent and connect one Android device. Inspect the existing installation and preserve unrelated files, credentials, enrollments, and agent settings. Use the official release or repository source, verify checksums when downloading, and run the matching au setup command once. Keep the setup local; do not use raw ADB or bypass Android prompts. If the device is not authorized, tell me to unlock it, enable Developer options and USB debugging, reconnect it, and accept the USB debugging prompt, then wait. When Android asks for accessibility, tell me to open Settings, Accessibility, Android Use, turn it on, and approve the warning; then resume with au doctor. Configure a local stdio MCP server with the absolute au path and arguments serve --mcp. Verify with android.read command status and android.read command screen. Use the new command-string tools for normal work, and never replay a partial or unknown mutation.
```

## Host install

From a verified release archive:

```powershell
.\au.exe doctor
.\au.exe setup
```

```sh
./au doctor
./au setup
```

If the public package is not confirmed, do not guess an npm release. Download the matching official archive, `SHA256SUMS`, and `release-manifest.json`; verify them before extraction. If Platform-Tools are installed elsewhere, set `AU_ADB` to the trusted executable.

## Device approvals

The device must be Android 8 or newer, unlocked, USB-debugging authorized, and the only enrolled hardware. `au setup` installs or updates the helper. Android still owns Accessibility, camera, microphone, notifications, location, and screen-recording approvals. Grant optional permissions only when the task needs them.

## Recovery

Run `au doctor` after every approval or connection change. Follow its `phase`, `next_step.kind`, ordered steps, and `resume` command. `agent` steps are host work; `user` steps require the Android device; `computer` steps repair a host dependency; `ready` means start the local MCP server. Use `au repair PATH` for a known helper APK and `au update` for a bundled update. Do not repeat an uncertain device mutation.

## Skill and MCP

The source skill is [`skills/android-use/SKILL.md`](../../skills/android-use/SKILL.md). Register that file with the agent's native skill installer, or keep it in the project and tell the agent to read it. Configure a local stdio server with the absolute executable and `serve --mcp`; preserve other MCP entries and reload the client. The server advertises only `android.read` and `android.act`, each with one required `command` string.
