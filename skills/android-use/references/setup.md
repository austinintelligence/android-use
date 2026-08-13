# Setup reference

If `au` is absent, run `npx --yes android-use@latest setup --agent auto --wait`. The installer selects the verified Windows/macOS/Linux x64/ARM64 host, installs compatible official platform-tools when available, and invokes the resumable host/platform-tools/device/helper/agent state machine. Use `au ready` for a read-only readiness check and `au doctor --repair` only for AU-owned repair. The persisted setup journal is under the platform-specific AU state directory as `state/setup.json`.

The state sequence is `HOST_INSTALLED`, `PLATFORM_TOOLS_READY`, `DEVICE_DETECTED`, `DEVICE_AUTHORIZED`, `DEVICE_ENROLLED`, `BRIDGE_INSTALLED`, `SEMANTIC_ACCESS_ENABLED`, `AGENT_CONFIGURED`, and `READY`. Multiple devices require an explicit endpoint; `unauthorized` means wait for the Android RSA dialog. `au setup --wait` polls only the bounded device/semantic transitions for two minutes; it never bypasses Android authorization or changes unrelated settings.

Optional capabilities are reported independently: notification access, camera,
microphone, location, and future remote access. They are not requested as a
single permission bundle. The helper setup screen exposes each capability as
an explicit user action, while `au doctor --json` reports the machine-visible
state.
