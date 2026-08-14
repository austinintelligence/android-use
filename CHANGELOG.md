# Changelog

## 1.0.0 — 2026-08-14

The first supported public release of Android Use.

- Control one enrolled Android device through the `au` CLI, MCP, or JSONL.
- Read compact semantic UI, act through generation-checked plans, control supported Chrome sessions, and keep screenshots and other large results as local artifacts.
- Install the matching Android helper with `au setup`, then use `au doctor` for clear connection and permission diagnostics.
- Official release archives are available for Windows x86_64, macOS Apple Silicon, and Linux x86_64, with a matching Android helper APK.
- Release assets include SHA-256 checksums, SPDX SBOM data, and GitHub build attestations.

### Important notes

- Android 8 or newer, USB debugging, and a data-capable USB cable are required for the supported onboarding path.
- Wireless ADB is an advanced transport, not the default setup path.
- Camera, microphone, location, notifications, screen recording, and browser access remain device- and permission-dependent.
- The project does not ship a remote broker, fleet manager, or multi-user authorization layer. Keep `au` and ADB private.
