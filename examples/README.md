# Examples

Send each command as the required `command` string to the matching MCP tool.

```text
android.read: status
android.read: screen
android.act: open app "DISPLAY NAME" then wait for "EXPECTED LABEL" up to 5 seconds
android.act: tap "TARGET"
android.act: page open "https://example.invalid" then page wait for text "EXPECTED TEXT" up to 10 seconds
android.read: page text matching "SEARCH TEXT"
android.act: page click "TARGET"
android.act: capture screen
```

Use labels, not guessed coordinates. If a target is duplicated, use the numbered command Android Use gives you. If a result is partial or unknown, read first and reconcile; never replay blindly.

For the compatibility CLI, `au serve --jsonl` accepts the old structured envelopes. Those examples are intentionally kept in [agent protocol](../docs/reference/agent-protocol.md), not the primary path.
