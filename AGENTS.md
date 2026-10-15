# Android Use agent guide

Use the local MCP server `au serve --mcp` ([client setup](docs/agents/clients.md)): two tools, `android_read` and `android_act`, each taking one `command` string. Without MCP, run `au read screen` or `au act 'tap "LABEL"'`. Every command form: `android_read help`.

Act directly on a label you expect, quoted exactly; read `screen` (or `page`) only when the state is unknown: `tap "Wi-Fi"`, `type "TEXT" in "FIELD"`, `scroll down`, `open app "NAME"`. A successful action reports the new screen, so do not read again. Join steps with `then`; use `number N` for a duplicate label. Use `tap point X Y` on the attached image only after a label is missing.

`stale`: retry. `partial` or `unknown`: read and reconcile before any other change. Ask before destructive, account, purchase, submission, notification, location, camera, microphone, or recording actions.

Details: [agent protocol](docs/reference/agent-protocol.md), [setup](docs/agents/quickstart.md), [security](SECURITY.md).
