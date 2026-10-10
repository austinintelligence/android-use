# Changelog

## 1.2.0 — 2026-10-10

**Highlights.** The on-device app is now called **Android-Use**, so the setup steps name what you see. `au` ships for six platforms (macOS and Linux and Windows, each on x86_64 and arm64), emulators are a first-class way to use it (`au emulator install` fetches the SDK pieces, and `cargo xtask emulator-e2e` checks the whole workflow), and the documentation and release process were rebuilt.

### Changed

- The helper app, its accessibility service, and its notification reader are labeled **Android-Use** (they were "AU Bridge", "AU semantic Android control", and "AU notification reader"). Host messages and docs now say "Settings → Accessibility → Android-Use". Upgrading with `au update` renames them in place.
- `au help` lists `install`, `uninstall AGENT`, `emulator`, `targets`, and `--target`.
- The install scripts install the newest release (they were fixed on 1.0.0) and support every platform below. `ANDROID_USE_VERSION` still pins a version.
- The README, install guide, and docs index are rewritten around one install path per platform, measured results, and the emulator workflow. Docs moved into `guides/`, `reference/`, `concepts/`, and `contributing/`; the internal planning documents were replaced by [measurements](docs/design/measurements.md).

### Added

- **Platforms:** macOS Intel, Linux arm64, and Windows arm64, beside macOS Apple Silicon, Linux x86_64, and Windows x86_64. `install/platforms.json` is the one list; `cargo xtask version` fails if the release workflow or an install script disagrees. macOS Intel and Windows arm64 are cross-built.
- `au emulator install` runs Google's `sdkmanager` for the missing emulator package and system image, attached to your terminal so you read and accept the SDK license.
- `cargo xtask emulator-e2e`: create, start, set up, read and act, a refused second serve, restart with enrollment intact, snapshot save and restore, in one run.
- npm: a README and metadata for `android-use`, six `@androiduse/<platform>` packages, and a publisher that skips packages already published (`install/publish-npm.mjs`).
- `install/render-formula.mjs` renders the Homebrew formula from a release's `SHA256SUMS`.
- The release workflow can be run by hand as a dry run that builds and checks everything and publishes nothing. Release notes are generated from this changelog.
- Docs: environment and error-code reference, security model, token budget, testing and releasing guides, and a Markdown link check in `cargo xtask verify`.

### Fixed

- `au emulator stop` powers Android off from the inside before cutting power. A power cut could lose the installed helper or its Accessibility setting on the next boot. `start` now waits for the package manager after boot and for an emulator that is still shutting down, and identity checks give a just-restored or still-starting emulator 15 seconds before calling it foreign.
- Documentation named a command (`au capabilities`), an install state (npm "not published"), and an ADB port forward that no longer exist.

### Maintenance

- Budgets (formatted lines): Rust production 7,520, Java 3,450, automation 1,700, authored code 15,550. The increases cover `au emulator install` and the emulator, release, and publishing tooling.

## 1.1.2 — 2026-10-10

**Highlights.** Agents need about half the tool calls and a fifth of the input tokens: two tools, dense screen rows, `Changed:` diffs after an action, chains that cross screens, and a helper that waits for the screen to settle. One command now installs and registers Android Use with your agents, release binaries carry the Android helper, and local emulators are separate targets. It also fixes a helper crash on Android 14+ that shipped in 1.0.0.

The `v1.1.0` and `v1.1.1` tags exist but never produced a release (tags cannot be moved once pushed): the release workflow lacked `npm ci`, then rejected the larger Linux binary. 1.1.2 is the first release since 1.0.0. It contains breaking changes that should have been a major version; nothing was published to npm or Homebrew before it, so only people running 1.0.0 archives are affected.

### Breaking

- The MCP tools are named `android_read` and `android_act` (some model APIs reject dots in tool names). The dotted names are still accepted in `tools/call` but no longer listed.
- The legacy structured JSON protocol is gone: `au serve --jsonl`, `au observe`, `au browser`, `au capabilities`, `au location`, `au notifications`, `au visual`, `au artifact`, and `au act JSON`. Use `au read COMMAND` and `au act COMMAND`, which reach every capability. About 650 lines of host code were removed.
- Screens are dense: `sw Wi-Fi=on`, `edit Email="x"`, `list Feed`, plain names for buttons and text, `Save#2` for a duplicate (send `tap "Save" number 2`). Descriptive subtitles moved to `screen full`.
- A successful action says `Done.` and then the screen, or on the same screen only `Changed:` rows (`~` changed, `+` added, `-` removed).

### Cheaper and faster for agents

- Fixed per-turn tokens fall from 834 to 396 (−53%); calls per task from 5.9 to 2.6; input tokens per task from 5,356 to 1,186 (−78%); success from 64% to 98%. See [measurements](docs/design/measurements.md).
- `computer/src/spec.rs` is the one table of command forms; it generates the tool descriptions, `android_read help`, the installed protocol reference, and the grammar tests.
- `scroll … until "TEXT"` pages a list in the helper in one call. A read can end a chain (`tap "Save" then find "Saved"`). Results carry `structuredContent` `{status, changed}`.
- Icon-only controls are `e7` and targeted with `tap e7`, by a key (class, view id, bounds); screen images carry a numbered box per control.
- A label that only contains the request (`tap "Save"` for "Save draft") is used when plainly meant (word-aligned, at most two other words, none risky), and the result says so.
- Every successful action reports the settled screen. Commands chain across screens: steps after an app, link, or page opens are resolved on what it shows. Screen images pass through content-aware compression (20 to 200 KB, exact coordinates mapped back to device pixels).
- The helper waits for the screen to react and go quiet inside the plan, so the host polls nothing. The helper link and Chrome use the adb server's own protocol: no `adb` process, no port forward. A cold `au read status` fell from 150 ms to 70 ms. Artifacts download in 256 KiB slices (they were 2.8 KB).

### Added

- `au install [AGENT|all] [--dry-run] [--force]` and `au uninstall AGENT` register `serve --mcp` with Claude Code, Codex, Cursor, Gemini CLI, VS Code, Claude Desktop, and OpenCode (and the skill for Claude Code and Codex). Only the `android-use` entry changes; the old file is saved as `.au-backup`.
- `au setup` installs the helper, registers every agent it finds, and ends with a live read of the screen.
- Release binaries embed the helper APK (SHA-256 checked), so `au` alone is enough. `npx -y android-use setup`, install scripts that verify `SHA256SUMS`, a Homebrew formula, and a Claude Code plugin and marketplace manifest are part of the repository.
- **Emulators and targets:** `au emulator doctor|images|create|start|stop|status|list|snapshot|destroy` runs local emulators without a JDK or `avdmanager`. `--target NAME` (or `AU_TARGET`) gives each its own enrollment, journal, artifacts, and exclusive `serve` lock; `au targets` lists them. An emulator is enrolled by a per-target identifier, not its port, and every action on the recorded port re-checks it, so a stale record cannot stop or bind another emulator. See [emulators](docs/guides/emulators.md).
- `au read COMMAND` and `au act COMMAND` give shell-only agents the same grammar. `au serve --mcp` starts without a device and reports problems as tool results with one recovery step. Errors name the failed step, whether anything changed, and the next safe move.
- `cargo xtask benchmark-live` drives the shipped commands over MCP and reports calls, tokens, time, and success per task, with optional gates. The `aubench` fixture gained duplicate, near-miss, slow, and unlabeled controls.

### Fixed

- The helper crash-looped on Android 14 and newer (`startForeground` without a type claims `mediaProjection` before consent); it now starts as `specialUse`. Android 14+ also hid views marked accessibility-data-sensitive (such as the Wi-Fi switch); the service now declares `isAccessibilityTool`.
- Chrome: `page click`, `focus`, and `type` always reported `stale` (the fingerprint was compared still JSON-quoted). Chrome tab lists show only pages.
- Notifications: `notifications` always said none, and `open`, `dismiss`, and `run notification action` could never succeed (the agent's label was sent where Android's key belongs). They now list by title and text and resolve labels to keys. `location` and `capabilities` return values instead of field names.
- Journal and recovery: operation ids are salted per process (a restarted client no longer collides with the durable journal); a pending record left by a crash fences new actions as `unknown` until the agent reads the screen; a lost outcome is `unknown`, not "nothing changed"; a plan that commits a step and then throws is `partial`. A stale plan is retried once, which is safe because `stale` proves nothing ran, and a missing label, failed verify, or stale receipt stops the remaining steps.
- Targeting: a label no longer taps the nearest unrelated control (possibly a destructive one); only a control containing the label's center is used. Labels match across Unicode spaces and brackets, duplicates are numbered top to bottom to match the listing, and `verify` and `wait` match labels and hints the way targeting does. Plain-text lists are one row per line. A screen image is never reused after a later change in the same command.
- Setup and status: two transports reporting one serial are refused; setup waits for the device to finish booting; `au doctor` asks the helper instead of trusting that it is installed; `au status` waits through the service rebinding and no longer reports ready while Accessibility is off.
- Protocol: unknown MCP methods return JSON-RPC `-32601`, `ping` is answered, recordings and audio are no longer sent as images, a failed fetch after a successful action no longer turns the result into an error, and `page scroll` is bounded to ±20,000 px.

### Maintenance

- `engine.rs` is split into `engine.rs`, `agent.rs`, `journal.rs`, `resolve.rs`, and `format.rs`. Helper Java is formatted with google-java-format (AOSP), checked in CI. A fuzzer exercises the command grammar, and agent flows run against a scripted fake helper that speaks the real wire protocol.
- Product code may not use `uiautomator`. Release artifacts are size-gated: host binary at most 1,000,000 bytes (1,300,000 on Linux, whose binaries are about 30% larger), helper APK at most 60,000 (now 39,200, from 52,454, by dropping Kotlin metadata).
- New docs: per-client MCP setup, result semantics, architecture with a request flow and module map, and emulators.
- Dependency: `serde_json` with `preserve_order` (adds `indexmap`), so editing an agent's configuration keeps its key order.

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
