# Android Use agent guide

Android Use gives an AI agent bounded control of one enrolled Android device. The preferred interface is the MCP server:

```text
au serve --mcp
```

Use `au serve --jsonl` only when the client does not support MCP. Do not drive the machine through raw ADB when the typed interface can complete the task.

## Operating loop

1. Read `android.read` with `q=status`.
2. Read `q=observe` for the semantic UI frontier.
3. Act with `android.act`, passing the returned `g` generation and a unique operation `id`.
4. Prefer integer refs from the latest observation. Keep plans short and linear.
5. Include an immediate `wait` or `assert` when the outcome matters.
6. On `stale`, observe and rebuild. On `partial` or `unknown`, observe before doing anything else. Never replay a mutation blindly.

For Chrome content, read `q=browser` with `op=tabs|observe|text`, then use a browser-targeted plan. Use Android UI semantics for Chrome's own toolbar.

Use semantic UI before screenshots. Request a screenshot only when layout, imagery, or an unlabeled control matters. Artifacts are private handles; fetch only the required range.

Ask before deletion, account changes, purchases, submissions, camera or microphone capture, location-sensitive work, notification actions, or screen recording.

Setup and recovery: [docs/agents/quickstart.md](docs/agents/quickstart.md)  
Typed protocol: [docs/reference/agent-protocol.md](docs/reference/agent-protocol.md)  
Security boundaries: [SECURITY.md](SECURITY.md)
