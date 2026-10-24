# CLI reference

Run `au setup` once with one authorized, unlocked Android device. Use `au doctor` when anything needs attention. Output is human-readable in a terminal and JSON otherwise; force either with `--human` or `--json`.

| Command | Purpose |
| --- | --- |
| `au setup [APK]` | Enroll the device, install or update the helper, and open the Accessibility screen. |
| `au doctor` | Explain missing device, helper, or permission state, with a resumable `next_step`. |
| `au status` | Quick readiness check; also notes battery optimization and TalkBack. |
| `au read COMMAND` | Same as the `android_read` tool, e.g. `au read screen`, `au read 'find "Wi-Fi"'`. |
| `au act COMMAND` | Same as the `android_act` tool, e.g. `au act 'tap "Wi-Fi"'`. Images are saved and returned as a path. |
| `au serve --mcp` | MCP server over stdio for agent clients. |
| `au test FILE... [--junit PATH]` | Run end-to-end test files written in the command grammar; exit 1 on a failure. See [testing apps](../guides/app-testing.md). |
| `au trace list\|show ID\|export ID\|to-test ID` | Review a session, export it as one page, or turn it into a test. |
| `au secret add NAME --apps PKG\|list\|remove NAME` | Keep a value in the system store for `type secret`. The value is read from stdin. See [control](../guides/control.md). |
| `au watch [--port N]` | A loopback page with the live command log and Pause, Resume and Stop. |
| `au devices` | List connected device endpoints and their states. |
| `au enroll ENDPOINT` | Bind one connected hardware identity when several devices are attached. |
| `au repair [APK]` / `au update [APK]` | Reinstall or update the helper. |
| `au uninstall [--dry-run]` | Remove the helper and Android Use's local state. `--dry-run` only lists what would go. Stop `au serve` first; uninstall refuses while it runs. Unknown flags are errors. `au uninstall AGENT [--dry-run] [--force]` removes one agent's entry; `--force` also removes an entry au did not write. |
| `au emulator doctor\|install\|images\|create\|start\|stop\|status\|list\|snapshot\|destroy` | Create and run emulators as separate targets. See [emulators](../guides/emulators.md). |
| `au targets` | List the default target and each emulator target. |
| `--target NAME` | With any command, act on that target instead of the default. `AU_TARGET` is the same. |
| `au version`, `au help` | Version and command list. |

## Environment

| Variable | Effect |
| --- | --- |
| `AU_ADB` | Absolute path of a trusted `adb` executable. Otherwise `ANDROID_SDK_ROOT`/`ANDROID_HOME` platform-tools, then `PATH`, are searched. |
| `AU_IMAGE_MAX_EDGE` | Upper limit on the longest side of screen images, in pixels (256–4096). Each screen uses the lower of this and its own limit: 1568 dense text, 1280 ordinary UI, 960 sparse. |
| `AU_IMAGE_BUDGET_KB` | Target size of a screen image in KB (32–4096). Default 350; fewer colors, then fewer pixels, are used to meet it. |
| `AU_HOME` | Directory for local state (enrollment, journal, artifacts). Defaults to `%LOCALAPPDATA%\AndroidUse` on Windows and `$XDG_STATE_HOME/android-use` or `~/.local/state/android-use` elsewhere. |

Grammar and results: [agent protocol](agent-protocol.md).
