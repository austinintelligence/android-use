# CLI reference

Run `au setup` once with one authorized, unlocked Android device. Use `au doctor` for readiness and `au status` for a quick check. Start an agent transport with `au serve --mcp`, or use `au serve --jsonl` when MCP is unavailable.

| Command | Purpose |
| --- | --- |
| `au devices` | List connected device endpoints and states. |
| `au enroll ENDPOINT` | Bind one connected hardware identity. |
| `au setup [APK]` | Install or update the helper and guide approvals. |
| `au doctor` | Explain missing device, helper, or permission state. |
| `au status` | Report readiness. |
| `au observe [BASE] [--detail]` | Legacy semantic read. |
| `au browser tabs\|observe\|text` | Legacy Chrome reads. |
| `au act JSON` | Legacy generation-checked action plan. |
| `au artifact ID [START END]` | Legacy bounded artifact read. |
| `au repair [APK]` | Repair the installed helper. |
| `au update [APK]` | Install a helper update. |
| `au uninstall` | Remove Android Use and its own local state. |
| `au version` | Print the version. |

Agents should use the two MCP command tools rather than constructing legacy JSON. Legacy fields remain for compatibility; see [agent protocol](agent-protocol.md).
