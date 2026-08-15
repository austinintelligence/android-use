# Setup

For a normal installation, run `au setup`. With one authorized Android device connected, it enrolls the hardware serial, installs or updates the helper, starts it, and reports the one Android approval still needed. On the device open `Settings → Accessibility → Android Use` and turn it on, then run `au doctor`.

For agents, prefer `au doctor --json` and `au setup --json`. Their `phase` and `next_step` fields are the setup state machine: follow the ordered steps, pause only when `next_step.kind` is `user`, and resume with the returned `resume` command. Do not invent a new recovery flow or repeat a failed device mutation.

Use `au status` for a quick readiness check, `au doctor` for recovery guidance, `au update` to refresh the helper, and `au uninstall` to remove only Android Use and its own local state. `au repair PATH` reinstalls a specific APK.

Advanced users can run `au enroll ENDPOINT` when more than one device is connected. The endpoint is only a transport selector; the enrolled hardware serial remains the identity check.

Start a persistent agent session with `au serve --mcp` or `au serve --jsonl` after the doctor reports ready.
