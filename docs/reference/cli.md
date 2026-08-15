# CLI reference

`au` is friendly in a terminal and JSON-first when piped to software. Use `--human` or `--json` to choose explicitly.

Setup and doctor JSON responses include `phase` and `next_step`. The next step has a `kind` (`agent`, `computer`, `user`, or `ready`), an ordered `steps` list, and a `resume` command so an agent can pause for an Android-owned approval and continue without guessing.

From a release archive, run `.\au.exe` in Windows PowerShell or `./au` on macOS/Linux. The shorter `au` examples below assume the archive folder has been added to your `PATH`.

## First-day commands

| Command | Purpose |
| --- | --- |
| `au devices` | List ADB endpoints and their connection state. |
| `au setup [APK]` | Enroll one device, install the helper, and report the remaining Android approval. |
| `au status` | Return readiness, UI generation, and capability mask. |
| `au doctor` | Explain required and optional checks in plain language. |
| `au observe [BASE] [--detail]` | Read the current semantic UI; optionally request a delta from a prior observation token. |

## Device and browser reads

| Command | Purpose |
| --- | --- |
| `au browser tabs` | List Chrome tabs and the selected tab. |
| `au browser observe` | Return the selected page's compact interactive frontier. |
| `au browser text` | Return bounded page text. |
| `au capabilities` | Report available optional capabilities and permission state. |
| `au location` | Read bounded current location data. |
| `au notifications` | Read compact notifications when access is enabled. |
| `au visual hash ID` | Compute a structural hash for a PNG artifact. |
| `au visual diff ID ID` | Compare two PNG artifacts with bounded sampled metrics. |

## Agent transports

| Command | Purpose |
| --- | --- |
| `au serve --mcp` | Run the MCP server over stdio. Preferred for agents. |
| `au serve --jsonl` | Run the typed JSONL adapter over stdio. |

## Typed actions and artifacts

| Command | Purpose |
| --- | --- |
| `au act JSON` | Execute one generation-checked Android, browser, or visual plan. |
| `au artifact ID [START END]` | Fetch a bounded byte range from a private artifact as base64 JSON. |

`au act` is intentionally low-level. Human users should normally let an MCP-connected agent form plans; integration authors should use the [agent protocol](agent-protocol.md).

## Maintenance

| Command | Purpose |
| --- | --- |
| `au enroll ENDPOINT` | Bind Android Use to a specific currently connected ADB endpoint. |
| `au repair [APK]` | Re-run helper installation and readiness repair. |
| `au update [APK]` | Install the bundled or supplied helper update. |
| `au uninstall` | Remove the helper from the enrolled device and delete Android Use local state. |
| `au version` | Print the CLI version. |

`uninstall` does not remove unrelated Android tools or device files.
