# Changelog

## 1.0.0 — 2026-08-14

The first supported public release of Android Use.

### Agent command interface

- Added the bounded model-facing `command` string for the existing `android.read` and `android.act` tools. The host now owns observation generations, operation identity, semantic target resolution, app and tab selection, safety limits, journals, and image content.
- Added plain-language receipts, ambiguity guidance, filtered page text, direct MCP image content, semantic-miss screenshots, allowlisted settings, safe links, point fallback, and bounded swipes.
- Kept the structured CLI, JSONL, MCP, golden-wire, helper, artifact, browser, and visual forms operational as a deprecated compatibility path. Raw generations, refs, plans, artifact ranges, and package IDs are legacy-only for ordinary agents.
- Browser actions reuse the active CDP connection, avoid unnecessary tab-list synchronization, track same-page DOM identity, use framework-friendly value events, and invalidate on meaningful DOM changes. Android text, content-description, and state changes invalidate semantic state.
- No new runtime dependency or cloud service was added. The helper remains local-only, authenticated, bounded, no-root, and without `INTERNET` permission.
- The automation source budget is now 1,250 lines (measured baseline 1,202) to cover the repository-owned documentation budget, parser-consistency, benchmark, and evaluation-status gates; production and authored-code limits remain unchanged.

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
