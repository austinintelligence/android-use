# Android Use agent guide

Use the local MCP server `au serve --mcp`. It exposes exactly two tools, each with one required string: `android.read` for non-mutating commands and `android.act` for bounded actions.

Read with commands such as `status`, `screen`, `page`, or `page text`. Act with runtime values such as `tap "TARGET"`, `type "TEXT" in "FIELD"`, `open app "DISPLAY NAME"`, or `page click "TARGET"`; nothing is typed unless the command requests it. Join a short sequence with `then`. The host owns observations, identity, target resolution, safety limits, journals, tabs, and image transport.

Read only when state is unknown. Prefer semantic labels. Use screenshots and `tap point X Y` only after a semantic miss. Retry only a stale pre-send failure; after `partial` or `unknown`, read and reconcile before mutating again. Ask before destructive, account, purchase, submission, notification, location, or camera/microphone/recording actions.

For setup see [quickstart](docs/agents/quickstart.md); for grammar see [agent protocol](docs/reference/agent-protocol.md); for security see [SECURITY.md](SECURITY.md).
