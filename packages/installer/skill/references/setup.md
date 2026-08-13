# Setup reference

If `au` is absent, verify the current distribution before choosing a host install path. Query npm instead of assuming `android-use@latest` exists. Check GitHub release assets for an exact OS/CPU match and do not treat a prerelease as stable. If neither source provides a supported host, report the blocker. The skill-only installation is `npx skills add austinintelligence/android-use --skill android-use -g -a codex -y`.

Once a verified host is installed, use `au setup --agent auto --wait`. It detects the device, stages the helper when available, connects the agent, and resumes safely after user-required Android prompts. Use `au ready --json` for a read-only check. Use `au doctor --repair --json` only to repair AU-owned state. The setup journal is stored as `state/setup.json` under the platform-specific AU state directory.

The state sequence is `HOST_INSTALLED`, `PLATFORM_TOOLS_READY`, `DEVICE_DETECTED`, `DEVICE_AUTHORIZED`, `DEVICE_ENROLLED`, `BRIDGE_INSTALLED`, `SEMANTIC_ACCESS_ENABLED`, `AGENT_CONFIGURED`, and `READY`. Multiple devices require an explicit endpoint. `unauthorized` means the user must approve USB debugging on the unlocked device. `au setup --wait` polls bounded device and semantic transitions for two minutes; it never bypasses Android authorization or changes unrelated settings.

If Android has not exposed USB debugging, guide the user through these standard steps: open Settings, open About phone/tablet, tap Build number seven times, return to Developer options, and enable USB debugging. Labels vary by manufacturer, so ask the user to search Settings for the named item when it is not in the expected place. Never use ADB to enable or pre-authorize debugging.

Finish with `au ready --json` and a read-only semantic observation. The proof must show the exact enrolled hardware identity, an authorized endpoint, helper readiness, and a bounded current-screen observation. Do not open a personal app, submit anything, or mutate user data merely to prove setup.

Optional capabilities are reported independently: notification access, camera,
microphone, location, and future remote access. They are not requested as a
single permission bundle. The helper setup screen exposes each capability as
an explicit user action, while `au doctor --json` reports the machine-visible
state.
