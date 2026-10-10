# Environment, files, and results

## Environment variables

| Variable | Used by | Effect |
| --- | --- | --- |
| `AU_HOME` | `au` | Directory for local state (enrollment, journal, artifacts, targets). Default: `%LOCALAPPDATA%\AndroidUse` on Windows, else `$XDG_STATE_HOME/android-use` or `~/.local/state/android-use`. |
| `AU_TARGET` | `au` | Target to act on, the same as `--target NAME`. Default: your enrolled device. |
| `AU_ADB` | `au` | Absolute path of a trusted `adb`. Otherwise `ANDROID_SDK_ROOT`/`ANDROID_HOME` platform-tools, then `PATH`. |
| `ANDROID_HOME`, `ANDROID_SDK_ROOT` | `au emulator` | Android SDK location (also searched in the usual Android Studio folders). |
| `AU_IMAGE_MAX_EDGE` | `au` | Longest side of screen images in pixels (256–4096). Each screen uses the lower of this and its own limit: 1568 dense text, 1280 ordinary UI, 960 sparse. |
| `AU_IMAGE_BUDGET_KB` | `au` | Target image size in KB (32–4096). Default 350. |
| `AU_BIN` | npm launcher | Use this `au` instead of the packaged one (for testing a local build). |
| `ANDROID_USE_VERSION`, `AU_INSTALL_DIR` | install scripts | Install a specific release; install somewhere other than the default folder. |
| `AU_EMBED_APK` | building `au` | Path of the helper APK to compile in; set by the release build. |

Development variables (`BENCH_*`, `E2E_*`, `AU_GRADLE`) are in [testing](../contributing/testing.md).

## Files

Everything lives under `AU_HOME`. The default target keeps `device.json` (the enrolled device), `operations.jsonl` (the journal), and `artifacts/`. An emulator target `NAME` keeps the same under `targets/NAME/`, plus `target.json`, `instance` (its identity), and `emulator.log`; emulator disks are under `emulators/avd/`. `au uninstall` removes the default target's state. `au emulator destroy NAME --yes` removes an emulator's.

## Output

On a terminal, `au` prints plain text; otherwise (or with `--json`) one JSON object per command: `{"ok":1,…}` on success, `{"ok":0,"e":CODE,"m":MESSAGE}` on failure. Force either form with `--human` or `--json`. Exit status is 0 on success and 1 on failure.

## Error codes

| `e` | Meaning |
| --- | --- |
| `args` | The command or its arguments were not understood. |
| `device` | No usable device or ADB: connect, unlock, or fix the SDK. |
| `identity` | The device is not the one enrolled for this target. |
| `helper`, `auth`, `sequence`, `protocol` | The on-device helper did not answer correctly; `au doctor` and `au repair` fix it. |
| `stale` | The screen changed before the action; nothing ran. Retry. |
| `ambiguous`, `missing` | A label matched several items, or none. |
| `partial`, `unknown` | Some steps ran, or the host cannot prove whether they did. Read before changing anything. |
| `timeout`, `bounds` | A deadline or size limit was hit. |
| `permission` | Android denied a capability; the user grants it on the device. |
| `unsupported`, `artifact`, `io` | The device lacks a feature; an artifact was missing; a local file error. |
