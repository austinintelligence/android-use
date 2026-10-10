# Connect an agent client

Android Use is a local stdio MCP server: command `au`, arguments `serve --mcp`. It starts even when no device is connected; tool results then explain what to fix.

## One command

```bash
au install            # every agent found on this computer
au install cursor     # one agent: claude, codex, cursor, gemini, vscode, claude-desktop, opencode
au install --dry-run  # show what would change; write nothing
au uninstall cursor   # remove only the android-use entry
```

`au setup` runs `au install` for you after the device is ready.

- Only the `android-use` entry changes. Other entries, key order, and indentation are kept, and the previous file is saved as `<file>.au-backup`.
- An `android-use` entry that `au` did not write is left alone unless you pass `--force`.
- Claude Code and Codex are configured through their own `mcp add` command (their files are rewritten by the running app), and get the skill in `~/.claude/skills` or `~/.codex/skills`.
- A config file with comments or trailing commas (VS Code and OpenCode allow them) is not edited; `au install` prints the snippet to paste instead.
- Desktop apps (Cursor, VS Code, Claude Desktop) do not inherit your shell's PATH, so they get the absolute path of `au` as found on PATH. Terminal agents get plain `au`.

Run `au setup` once first (see the [install guide](../guides/install.md)). The manual forms follow; replace `au` with an absolute path if the client cannot find it.

## Claude Code

```bash
claude mcp add android-use --scope user -- au serve --mcp
```

Or commit a project-scoped `.mcp.json`:

```json
{ "mcpServers": { "android-use": { "command": "au", "args": ["serve", "--mcp"] } } }
```

## Codex

Add to `~/.codex/config.toml`:

```toml
[mcp_servers.android-use]
command = "au"
args = ["serve", "--mcp"]
```

## Cursor and Claude Desktop

Cursor reads `~/.cursor/mcp.json` (or `.cursor/mcp.json` in a project); Claude Desktop reads `claude_desktop_config.json`. Both use:

```json
{ "mcpServers": { "android-use": { "command": "au", "args": ["serve", "--mcp"] } } }
```

## VS Code

Add to `.vscode/mcp.json`:

```json
{ "servers": { "android-use": { "type": "stdio", "command": "au", "args": ["serve", "--mcp"] } } }
```

## Any other MCP client

Use a stdio transport with the command and arguments above. The server implements MCP revisions 2025-06-18, 2025-03-26, and 2024-11-05, answers `ping`, and sends operating `instructions` during `initialize`. It exposes two tools:

| Tool | Use |
| --- | --- |
| `android_read` | Look without changing anything: `screen`, `page`, `notifications`, … |
| `android_act` | Change the device: `tap "Wi-Fi"`, `type "hello" in "Message"`, `page open "https://…"`, … |

Each takes one string argument, `command`. (Older clients that call `android.read` and `android.act` still work; only the underscore names are listed.) The tool descriptions list the common commands, so an agent can work from them alone; the [agent protocol](../reference/agent-protocol.md) explains results in more depth.

## One agent per device

A server drives one target: your enrolled phone by default, or an [emulator](../guides/emulators.md) with `--target NAME`. Register it like any other server, with `args` `--target NAME serve --mcp` (for example `claude mcp add android-demo -- au --target demo serve --mcp`). A second server for the same target is refused.

## Agents without MCP

Any agent that can run shell commands can use the same grammar through the CLI. Output is JSON when stdout is not a terminal:

```bash
au read screen
au act 'tap "Wi-Fi"'
au act 'type "hello" in "Message" then tap "Send"'
```

Images an agent asks for (for example `au act 'capture screen'`) are saved under Android Use's private artifact folder and returned as a file path.

## Check the connection

Ask the agent to run `android_read` with `status`, then `screen`. A healthy setup answers `Ready. Android Use is connected…` followed by the visible controls. If not, `au doctor` on the computer explains the next step.
