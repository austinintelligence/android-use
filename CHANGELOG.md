# Changelog

## 1.1.2 — 2026-10-10

This is the first release since 1.0.0. The `v1.1.0` and `v1.1.1` tags exist but never produced a release, and tags cannot be moved once pushed. The release workflow did not install the npm dev tools its lint step needs (now `npm ci`), and the 1,000,000-byte binary limit, calibrated on macOS, rejected the larger Linux build (now 1,300,000 on Linux).

The npm launcher is published as `android-use`, with its platform binaries in `@androiduse/darwin-arm64`, `linux-x64`, and `win32-x64` (the `@android-use` scope was not available).

### Breaking

- The MCP tools are named `android_read` and `android_act`. Some model APIs reject dots in tool names. The dotted names are still accepted in `tools/call`, but are no longer listed.
- The legacy structured JSON protocol is removed: `au serve --jsonl`, `au observe`, `au browser`, `au capabilities`, `au location`, `au notifications`, `au visual`, `au artifact`, and `au act JSON`. Use `au read COMMAND` and `au act COMMAND`, which reach every capability. About 650 lines of host code are gone.
- Screens are rendered densely: `sw Wi-Fi=on`, `edit Email="x"`, `list Feed`, plain names for buttons and text, and `Save#2` for a duplicate (send `tap "Save" number 2`). Descriptive subtitles move to `screen full`.
- A fully successful action says `Done.` rather than restating the command, followed by the screen or, on the same screen, only `Changed:` rows (`~` changed, `+` added, `-` removed).

### Emulators and targets

- `au emulator doctor|images|create|start|stop|status|list|destroy` creates and runs local Android emulators without a JDK or `avdmanager`. Each is a separate target: `--target NAME` (or `AU_TARGET`) selects its own enrollment, journal, artifacts, and an exclusive `au serve` lock. `au targets` lists them. See `docs/emulators.md`.
- `au emulator snapshot NAME save|load TAG` saves and restores a named restore point.
- `emulator stop`, `status`, `start`, and `setup` verify the identifier on the recorded port first, so a stale record after a crash cannot stop or bind a different emulator. A startup that times out kills the emulator it spawned. `--target` without a name is an error instead of falling back to the default target.
- An emulator is enrolled by a per-target identifier, not its port, so it survives a restart on another port and is never confused with another device.
- Android Use still never enables Accessibility itself; provisioning that would skip Android's consent screen is deliberately not built.
- The Rust source budget rises from 7000 to 7450 lines (authored code 14500 to 15000) to cover about 430 lines of emulator code.

### Cheaper for agents

- Fixed per-turn tokens (instructions plus both tool definitions) fall from 834 to 396 (o200k, −53%), driven by `computer/src/spec.rs`: one table generates the tool descriptions, the new `android_read help`, the installed `references/protocol.md`, and the grammar tests. `SKILL.md` is 263 tokens instead of 548.
- Screen listings are about 36% smaller on recorded screens, and a result on an unchanged screen is a few rows instead of the whole screen. `cargo xtask docs` gates both numbers.
- `scroll up|down [in "AREA"] until "TEXT"` pages a list in the helper and stops at the end: one call instead of a scroll/read loop.
- A read can end a chain: `tap "Save" then find "Saved"` runs the action and returns what you looked for.
- Results carry `structuredContent` `{status, changed}`; errors carry `{status}`.
- Icon-only controls are listed as `e7` and targeted with `tap e7`, by a key (class, view id, bounds) remembered from the last listing. Screen images carry a numbered box over each control.
- A label that only contains the request (`tap "Save"` for "Save draft") is acted on when it is plainly meant (word-aligned, at most two other words, none destructive) and the result names it; otherwise it counts as missing.
- A label miss attaches the screen image only when the text is thin. Omitted-row counts come first, so truncation cannot hide them.

### Faster

- The helper waits for the screen to react and go quiet inside the plan (and, after a launch, for the app to be in front), so the host polls nothing and replies describe the screen the action led to.
- The helper link and Chrome DevTools use the adb server's own protocol directly: no `adb` process runs, no port forward exists to leak. A cold `au read status` fell from 150 ms to 70 ms.
- Plans split after a step that opens another screen or page (open app, link, setting, key, notification, `page open`), so later labels are resolved on the screen they belong to.

### One-command install

- `au install [AGENT|all] [--dry-run] [--force]` and `au uninstall AGENT` register `serve --mcp` with Claude Code, Codex, Cursor, Gemini CLI, VS Code, Claude Desktop, or OpenCode (and the skill for Claude Code and Codex). Only the `android-use` entry changes; other entries, key order, and indentation stay; a foreign entry needs `--force`; a file with comments is not edited (the snippet is printed).
- `au setup` installs the helper, registers every agent it finds, and ends with a live `status` and `screen` read.
- Release binaries embed `aubridge.apk` (SHA-256 checked), so `au` alone is enough. `npx -y android-use setup`, an install script that verifies `SHA256SUMS`, a Homebrew formula, and a Claude Code plugin and marketplace manifest are included in the repository; nothing is published yet.

### Fixed

- The helper crash-looped on Android 14 and newer: `startForeground` without a type claims `mediaProjection` before consent. It now starts as `specialUse`.
- Android 14+ hid views an app marks accessibility-data-sensitive, such as the Settings Wi-Fi switch. The service now declares `isAccessibilityTool`.
- `page click`, `page focus`, and `page type` always reported `stale`: the page fingerprint was compared still JSON-quoted. (Present in 1.0.0.)
- A list of plain text became one row with a long value; each line is now its own row, tappable by its bounds.
- Duplicate numbers, headings, nested rows, scroll targets, and fields resolve consistently between the listing and the helper.
- Two connected transports that report one serial are refused instead of the first being used; setup waits for the device to finish booting; `au doctor` asks the helper something instead of trusting that it is installed.
- `au status` waits through the service rebinding after an update instead of reporting Accessibility off.

### Maintenance

- `cargo xtask benchmark-live` drives the shipped commands over MCP on a device and reports calls, tokens, time, and success per task, with p50/p95 per action and optional gates (`BENCH_GATE=1`). The `aubench` fixture gains duplicate, near-miss, slow, and unlabeled controls. Baselines are in `docs/design/baseline/`.
- Product code may not use `uiautomator`. Release artifacts are size-gated: host binary at most 1,000,000 bytes (1,300,000 on Linux, whose binaries are about 30% larger), helper APK at most 60,000 (now 39,148, from 52,454, by dropping Kotlin metadata).
- Budgets (formatted lines): Rust production 7,450, Java 3,450, authored code 15,100, automation 1,400. Rust production is about 7,380 against 5,620 in 1.0.0 after removing about 650 lines of legacy protocol; the additions are the install command, the spec table, handles and marks, the journal, compression, the agent layer, and emulator targets. `install/publish-npm.mjs` publishes a release to npm.
- Dependency: `serde_json` with `preserve_order` (adds `indexmap`), so editing an agent's configuration keeps its key order.

### Fixed

- `notifications` always reported "no actionable notifications" because the host read the helper's notification object as an array. It now lists notifications by title and text, newest first.
- `open notification`, `dismiss notification`, and `run notification action` sent the agent's label where the helper expects Android's internal notification key, so they could never succeed. Labels now resolve to keys by title or text, with `number N` for duplicates.
- `location` and `capabilities` returned only field names ("available for lat, lon, …"). They now return the coordinates, accuracy, and age, and the readiness of each capability.
- Operation ids were derived from JSON-RPC request ids, which restart in every client session, while the journal is durable. After a client restart, actions could fail with "operation id was already used". Ids are now salted per server process.
- A label missing from the screen no longer lets later `then` steps run; the result says which steps did not run.
- A failed verify, wait, or stale receipt stops the command instead of continuing with the next group of steps.
- `au status` and `android.read status` reported ready while the Accessibility service was off.
- `type "TEXT" in "FIELD"` could not find a filled text field by its hint. The helper now matches hints after exact text and descriptions.
- Unknown MCP methods now return JSON-RPC `-32601`, and `ping` is answered.
- Recordings and audio are no longer sent as MCP `image` content.
- `page scroll` through the command grammar is now bounded to ±20,000 px, like the structured form.
- A stale plan is recompiled and retried once, which is safe because `stale` proves nothing ran. When an earlier step already ran, results never advise retrying the whole command.
- An image or file that cannot be fetched after an action succeeded no longer turns the result into an error, which could have led an agent to repeat the action.
- Horizontal scrolls never page a vertical list. A horizontal scroll needs a view that offers one, and otherwise fails with a clear message.
- Labels with brackets match as displayed (`Inbox 3` for `Inbox [3]`). A label with more than 16 matches reports ambiguity instead of a protocol error.
- `verify` and `wait for` accept field hints, like the target resolver.
- Legacy JSON `act` probes the helper connection before journaling, so an idle-closed socket is an error rather than a spurious `unknown`.
- A journal holding only pending records evicts the oldest instead of refusing all new work.
- A resent request id whose earlier attempt may have run is explained as such instead of reported as a malformed request.

- Journal: a pending record left by a crashed process fences new actions as `unknown` until the agent reads the screen, instead of letting an uncertain action be repeated. Records are written in one write and a torn tail is removed on open, so a later record can no longer be glued onto a damaged one. Pending records are never evicted. The journal is locked and reloaded on every access, so the CLI and the MCP server can share it. Atomic replacement renames over the old file on Unix, so a crash cannot leave no journal at all.
- An action whose outcome is lost is recorded as `unknown` rather than `nothing changed`, on Android and in Chrome. A Chrome click whose reply is lost is no longer reported as not applied, and creating or closing a tab that cannot be confirmed reports `unknown`.
- Android: a plan that commits a step and then throws reports `partial`, not `nothing ran`.
- Android: a label no longer targets the nearest control on screen when no control contains it. That fallback could tap an unrelated control such as a destructive button; now only a control containing the label's center is used.
- Verify and wait checks match labels and hints the way targeting does. A ref check fails closed once the screen has changed, instead of checking whichever node now has that index.
- Labels match across Unicode spaces (for example a non-breaking space before a colon) and a label truncated for display matches the item it names. Targeting and verification use one normalization on the host and the helper.
- Notifications are listed and targeted by one label: the title, or the text when untitled. A number shown in the listing selects the same notification.
- A partial result that stops at a missing label is reported as an error to MCP clients, not as success.
- A command may take at most one photo, recording, or screen capture, since the helper returns one artifact per plan. A camera photo is no longer treated as the screen image, and cropping or comparing a photo reports a clear limit instead of a helper failure.
- A screen image is never presented after a later change in the same command, and points are refused until the screen is captured again after any change. Coordinates from one image can no longer reach a changed screen.
- Chrome tab lists count and show only pages, with an explicit note when more exist.
- Screen compression classifies horizontally striped screens as detailed content, and 4K-class captures are no longer rejected for their pixel count.
- Duplicated Android labels are ordered top to bottom, which matches the screen listing, so the number shown selects the same control.

### Agent experience

- Every successful action reports `Now on screen:` with the settled screen (or the page after `page` actions), so agents need far fewer separate reads.
- Commands can chain across screens: in `open app "Settings" then tap "Wi-Fi"`, the steps before a label that is not yet visible run first as their own generation-checked plan, the UI settles, and the rest compiles against the new screen. Previously the whole command failed because "Wi-Fi" was not on the starting screen.
- `au serve --mcp` starts without a connected device or adb. Problems become tool results with one concrete recovery step instead of a server that fails to start.
- Errors now name the step that failed, whether anything changed, and the next safe move. Parse errors quote the failing step and suggest the canonical form for that verb.
- MCP tool descriptions include the full grammar, the server sends `instructions` on `initialize`, and protocol versions 2025-06-18, 2025-03-26, and 2024-11-05 are negotiated.
- New `au read COMMAND` and `au act COMMAND` give shell-only agents the same plain grammar. `au act` still accepts a legacy JSON plan.
- New `scroll up|down|left|right` with no target scrolls the screen's main list, preferring areas that advertise that direction. `type … into …` is accepted.
- Duplicate labels are shown as `Save (number 2) — button`, the exact form to send back.
- Screen images go through a content-aware compression pipeline (`compress.rs`): classify the screen, resize to what a vision model keeps anyway, quantize to an indexed PNG (exact for screens with ≤256 colors), and meet a 350 KB budget by trimming colors then pixels. A synthetic 1080×2400 UI capture drops from 1.2 MB to about 20 KB in roughly 25 ms. `AU_IMAGE_MAX_EDGE` and `AU_IMAGE_BUDGET_KB` tune it.
- The agent is told each image's size, scale, and the device size. `tap point` / `swipe` coordinates map through that record to the device pixel under the image pixel's center, the result confirms the pixel hit, and a point outside the image or a rotated screen gets a corrected-context message instead of a misplaced tap. The mapping persists across one-shot `au act` calls.
- `image hash` / `image difference` compare full-resolution captures, so differently compressed screens still compare correctly.
- The default `screen` summary is about 1,200 bytes instead of 480, so typical screens need one read.

### Speed and reliability

- Device artifacts download in 256 KiB slices instead of 2,800-byte slices: a full screenshot takes a handful of round trips instead of about a thousand. Hosts fall back automatically for older helpers.
- Reconnecting to an already-running helper skips `am start-foreground-service`. The handshake retries until the service is listening instead of racing it.
- Connection failures before dispatch are reported as errors instead of `unknown`, and no longer leave pending journal records.
- Journal completion records are no longer force-flushed. Pending records still are, so at-most-once safety is unchanged and each action does one fewer full-disk flush.
- The helper loads app labels once per app and caches the launcher list for 30 seconds.
- Read-only Chrome calls reconnect only on transport errors, not on every failure.
- The agent layer keeps a fixed set of named images instead of an ever-growing `image N` alias per capture.

### Architecture and maintenance

- `engine.rs` is split into `engine.rs` (execution core), `agent.rs` (agent-facing layer), `journal.rs`, `resolve.rs` (one matching rule for screens, pages, tabs, apps, and notifications), and a standalone `format.rs`. Legacy JSON parsing moved to `legacy.rs`.
- Helper Java is formatted with google-java-format (AOSP style), with a CI check. Source budgets now measure formatted code: Android production 3,200 lines and authored code 13,000.
- `cargo xtask size` now counts trailing `#[cfg(test)]` modules as tests, not production code.
- `api_tests.rs` moved to `computer/tests/command_grammar.rs`. End-to-end agent flows run against a scripted fake helper that speaks the real wire protocol. A new fuzzer exercises the command grammar; the previous fuzz test only reached the JSON parser.
- New docs: per-client MCP setup (`docs/agents/clients.md`), result semantics in the agent protocol, an architecture page with request flow and module map, and toolchain setup in development.
- Removed the unused `Error::trace` field and `visual::Op`.

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
