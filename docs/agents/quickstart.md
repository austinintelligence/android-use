# Agent quickstart

Install the host and helper with `au setup`, approve Android Use under Settings → Accessibility, then confirm with `au doctor`. Start a local stdio MCP server:

```text
au serve --mcp
```

The agent receives `android.read` and `android.act`, each with one required `command` string. Try:

```text
android.read: status
android.read: screen
android.act: tap "Settings"
```

Use `page ...` for Chrome content. Read only when state is unknown, act by label, and read after `partial` or `unknown`; never replay an uncertain mutation. For installation, permissions, and repair see [install](install.md). For exact grammar see [agent protocol](../reference/agent-protocol.md).
