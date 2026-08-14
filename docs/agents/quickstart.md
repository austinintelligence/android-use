# Agent Quickstart

Android Use is a device tool, not an autonomous agent. Connect it to the coding agent you already use.

## MCP setup

Add a local stdio MCP server whose command is the absolute path to `au` and whose arguments are:

```text
serve --mcp
```

The exact configuration file differs by client. The resulting tools must be named `android.read` and `android.act`. Restart or reload the client, then ask it:

```text
Check whether Android Use is ready. Observe the current screen, but do not change anything.
```

The agent should call `android.read` with `q=status`, then `q=observe`.

## Copy-paste instruction for an agent

```text
Read AGENTS.md in this repository. Check `au status`, then connect `au serve --mcp` as a local stdio MCP server. Observe the device without changing it. If setup is incomplete, run `au doctor` and tell me exactly which Android-controlled approval remains. Never replay a partial or unknown mutation.
```

## Efficient tool use

- Read semantic UI first. It is cheaper and more actionable than a screenshot.
- Reuse references only within the generation that returned them.
- Put related actions in one short plan and include the immediate expected result.
- Use Chrome's CDP view for page content; use Android semantics for Chrome's toolbar.
- Fetch artifact bytes only when the task needs the image, audio, or video.
- Do not poll unchanged state. On `stale`, read once and rebuild.

The authoritative behavior contract is [AGENTS.md](../../AGENTS.md). The wire shapes are in [Agent protocol](../reference/agent-protocol.md).
