# Agent installation and recovery

Use this page for installing Android Use, registering its agent skill, connecting one Android device, and recovering setup. For day-to-day device work, use the installed `android-use` skill instead.

## Copy-paste prompt

Paste this into Codex, Cursor, Claude Code, OpenClaw, Hermes, or another coding agent:

```text
Set up Android Use for this agent and connect one Android device.

Use https://github.com/austinintelligence/android-use as the source of truth. Work through the setup in order and keep the conversation on rails:

1. Inspect the computer, operating system, CPU architecture, existing `au` installation, Android platform tools, and current agent configuration. Reuse a working installation when possible. Do not delete or overwrite unrelated files, device data, agent settings, credentials, or an existing Android Use enrollment.

2. Register the `android-use` Agent Skill for the agent I am using. Prefer the agent's native skill installer. For a skills.sh-compatible agent, use the matching agent id with:
   `npx skills add austinintelligence/android-use --skill android-use -g -a <agent-id> --copy -y`
   For OpenClaw, use:
   `openclaw skills install git:austinintelligence/android-use@main --global`
   Replace `<agent-id>` with the real id; do not run it literally. Reload the agent if its skill list is cached.

3. Install the host runtime from an official source. First check whether `android-use` is actually published before using `npx android-use@latest`. If it is not published, download the matching archive from the latest official GitHub release, verify the archive against both `SHA256SUMS` and `release-manifest.json`, and extract it to a durable user-owned directory. Keep `au` and `aubridge.apk` together and use the absolute path to `au`. Do not use an unsigned or unexplained prerelease unless I approve it.

4. Check readiness with `<absolute-au-path> doctor --json`. If Android platform tools are missing, use an already installed trusted `adb` when available; otherwise tell me exactly how to install platform-tools or set `AU_ADB`. If no authorized device is found, do not keep retrying. Tell me, in plain language:
   - unlock the phone or tablet;
   - use a USB cable that carries data;
   - open Settings → About phone and tap Build number seven times if Developer options is not visible;
   - open Developer options and turn on USB debugging;
   - reconnect the device and tap Allow on “Allow USB debugging?”; choose Always allow only for my own computer.
   Then wait for me and rerun `doctor --json`.

5. Run `<absolute-au-path> setup --json` once the device is authorized. If it reports an Android permission step, tell me exactly what to tap: open Settings → Accessibility → Android Use, turn Android Use on, and approve Android's warning. Wait for me, then rerun `setup --json` or `doctor --json` to verify the change. If multiple devices are connected, show me their endpoints and ask me which one to enroll; never guess.

6. When `doctor --json` reports ready, connect the local MCP server using the absolute executable path and the arguments `serve --mcp`. Preserve other MCP entries, keep the server on local stdio, and reload the agent. Then verify with `android.read` using `q=status` followed by `q=observe` without changing the device.

At the end, report: the installed `au` path and version, the registered skill location, the enrolled device identity without exposing secrets, the MCP connection, required checks, optional capabilities, and the exact next action if anything is still waiting on me. If any step fails, read https://github.com/austinintelligence/android-use/blob/main/docs/agents/install.md and resume from the reported phase. Never bypass Android security prompts or replay an unknown device mutation.
```

The prompt intentionally separates computer work from Android-owned approvals. The agent should continue automatically after each approval instead of making you repeat the whole installation.

After setup succeeds, tell the agent what you want done on the device, such as: “Open Settings and tell me which Wi-Fi network is connected.”

## What the setup state means

`au doctor --json` and `au setup --json` return a `phase` and a `next_step` object when something still needs attention. `next_step.kind` is one of:

- `agent` — the agent can run the next command itself.
- `user` — Android or device hardware needs your hands.
- `computer` — a host dependency such as `adb` needs attention.
- `ready` — setup is complete and the next command is the local MCP server.

The `next_step` object includes a short title, ordered steps, and a `resume` command. Agents should report those fields instead of inventing a different recovery procedure.

## Manual fallback

If the agent cannot install a host runtime automatically:

1. Open the [official releases](https://github.com/austinintelligence/android-use/releases) page and choose the archive for the computer: Windows x86_64, macOS Apple Silicon, or Linux x86_64.
2. Download the archive, `SHA256SUMS`, and `release-manifest.json` from the same release.
3. Verify the archive before extracting it. On Windows PowerShell use `Get-FileHash`; on macOS/Linux use `shasum -a 256`.
4. Keep `au` and `aubridge.apk` in the same extracted directory.
5. Run the matching command from that directory:

```powershell
.\au.exe doctor --json
.\au.exe setup --json
```

```sh
./au doctor --json
./au setup --json
```

Do not use `npx android-use@latest` until `npm view android-use version` confirms that the public package exists. The npm launcher in this repository is prepared for publication but is not itself proof that npm publication has happened.

If `adb` is missing, install the official [Android SDK Platform-Tools](https://developer.android.com/tools/releases/platform-tools), reopen the agent terminal, and run `au doctor --json` again. If platform-tools already exists somewhere else, point Android Use at it with `AU_ADB` instead of installing a second copy.

## Register the skill manually

The skill source is [`skills/android-use/SKILL.md`](../../skills/android-use/SKILL.md). For common agents:

```console
# Codex
npx skills add austinintelligence/android-use --skill android-use -g -a codex --copy -y

# Cursor
npx skills add austinintelligence/android-use --skill android-use -g -a cursor --copy -y

# Claude Code
npx skills add austinintelligence/android-use --skill android-use -g -a claude-code --copy -y

# OpenClaw
openclaw skills install git:austinintelligence/android-use@main --global
```

Restart or reload the agent after installing the skill. If an agent has no Agent Skills support, keep the skill file in the project and tell the agent to read it before using Android Use.

## Connect MCP manually

Configure a local stdio MCP server with the absolute path to `au` and these arguments:

```text
serve --mcp
```

The server must remain local. Do not expose its stdio stream through an unauthenticated network bridge. After reloading the client, ask it to check status and observe without changing anything.
