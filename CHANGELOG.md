# Changelog

## 1.4.0 — 2026-10-25

**Highlights.** An agent can now keep what worked, be held back by a person's rules, and be watched and stopped from the computer or the phone. Commands, tools and flags from 1.3 are unchanged; everything new is additive and off until used. Design and test plan: [docs/design/1.4.md](docs/design/1.4.md).

### Upgrading from 1.2

1.3.0 was not published on its own, so 1.4.0 is the first release after 1.2.0 and includes all of it ([1.3.0 notes](https://github.com/austinintelligence/android-use/blob/v1.4.0/CHANGELOG.md#130--2026-10-24)). Its behavior changes apply when you upgrade:

- `au uninstall` flags now work as named: `--dry-run` only reports, unknown flags are errors, and uninstall refuses while `au serve` is running.
- Host state is private and artifacts expire after 7 days; a shared `AU_HOME` on CI must become per user.
- au upgrades the helper by itself, and going back to 1.2 needs `au uninstall` first.
- A label the agent never saw no longer runs if it is risky.
- `run notification action` needs a label when a notification has several actions.
- Newlines in screen text show as spaces, and `toggle` checks that the switch flipped.
- MCP results no longer carry `structuredContent` (see Fixed).

### Security

- The journal's plan digests are keyed with a random per-process key, so text typed into a field (a PIN, a password) cannot be recovered from `operations.*` by guessing; digests are only ever compared within one process.
- `policy.toml` covers the app on screen, not only `open app`: `open setting` counts as Settings, while a kept-out app is in front only `press back`/`press home` run, and a command that leads into one is answered with Home. A loose name match no longer counts as one of the app's names, and a failed app-list lookup never lets a denied app through. `capture.allow = false` also keeps screen images out of replies.
- While Android's own permission or install prompt is the active window, only `press back` and `press home` run, whatever the label, handle or point.
- `type secret` needs a field the helper's own flags mark as a password, with no plain field imitating one; replies hide any line that still holds a piece of the value, so shortened or cleaned-up copies cannot leak it.
- Phone approvals: the helper runs no actions while one waits, so no connection can press its buttons; it is not shown over the lock screen; steps longer than the 300 characters the phone shows are refused; Stop and pause are checked again after the wait.
- Trace redaction follows the grammar (`type`/`TYPE`, `in`/`into`, curly quotes, `eN` targets) and redacts routine values.
- Pulls copy regular files only, and transfers count against `actions.max_per_session`.

### Added

- Routines: `save routine "NAME" from last [with "VALUE" ...]`, `run routine "NAME" [with ...]` and `read routines` replay a command that worked, in one call, with values as parameters. A routine is text and runs through the normal parser, journal and risky-word rules.
- `policy.toml`: `apps.allow`/`apps.deny` (matched on name, package and launcher label), `risky.extra`, `actions.max_per_session`, `capture.allow`, `codes.apps`, `files.dir`. A bad file refuses the session. New result codes `blocked_by_policy`, `paused` and `denied`; `read policy` shows what is in force.
- Secrets: `au secret add|list|remove` (value on stdin, kept in the system store) and `type secret "NAME" in "FIELD"`, typed only into a single password field of an allowed app and removed from replies, errors, traces and routines.
- `ask phone "QUESTION" then STEPS`: a full-screen Allow once / Deny prompt on the phone that shows the exact steps, cannot be answered by the agent, and voids itself if the screen's labels change meanwhile (needs the 1.4 helper).
- Session traces (`au trace list|show|export|to-test`), written with password-like and clipboard text redacted and deleted after 7 days.
- `au test FILE... [--junit PATH]`: end-to-end tests in the command grammar with `expect screen`, `expect no screen`, `expect result`, `reset app` and `clear app`.
- `au watch`: a loopback page (token, loopback `Host` only) with the live command log and Pause, Resume and Stop.
- `wait for notification [from "APP"] up to N seconds`, `read code from notification "APP"` and `read audit`.
- `push file` / `pull file` between `files.dir` and the phone's Downloads (off without the policy key; plain names, 64 MB, no links out of the folder, no overwrite).
- Fixture app lab screens, eight more `benchmark-live` tasks, and `bench-model` support for refusal tasks, mid-run interference and per-family reports (`tools/bench/tasks-1.4.json`).

### Changed

- A changed screen is reported once it holds still for a poll (at most 700 ms more), with a note when it never does.
- A dialog or popup, the keyboard and Android's own permission prompts get one line above the listing when the helper has the `tree` op. The agent cannot tap Allow, OK or Accept while Android's own prompt is up.
- Runs of six or more look-alike plain rows (`Fixture row 1` ...) collapse to the first two, a `+N more like` line and the last; `screen full` lists them all. Risky, stateful and mixed-kind runs never collapse.
- A `Partly done` result includes the current screen.
- The npm launcher passes `test`, `trace`, `secret` and `watch` through to `au`; it refused them before.
- Policy, secrets, routines, traces and the `au watch` control file live in `AU_HOME` itself, also for emulator targets, where the `au secret`, `au trace` and `au watch` commands look.
- Windows secret files are named by the hex of the secret's name, and PowerShell gets the file path through the environment.
- Source budgets raised once for these features (Rust 12,000, Java 5,600, automation 3,900, authored 27,500), and the documentation budgets for the protocol and CLI references.
- The host binary stays under its size limit without raising it (Linux x86_64 1,445,376 bytes against 1,450,000): the session layer is compiled once instead of inlined into both the MCP loop and the CLI, window notes read the helper's `tree` reply as plain JSON instead of through generated deserializers, secrets metadata reuses the JSON value code, and sorts that did not need to be stable use one shared instantiation or none.

### Fixed

- The MCP tools no longer return `structuredContent` (contributed by @lisniuse in [#90](https://github.com/austinintelligence/android-use/pull/90)). The server declares no `outputSchema` but sent structured content with every result, and a client that prefers it to `content`, such as Claude Code, handed the model `{"status":"ok"}` in place of the screen: every read arrived empty and every failure lost its explanation. The status is already in the text. **Compatibility:** a client that read `structuredContent.status` or `.changed` no longer sees them. Present since at least 1.1.2.

### Not done

- Gradle dependency locking: generating the lockfiles needs Google's Maven repository, which the environment this release was prepared in could not reach.
- The helper and fixture Java was compiled against the Android 16 framework and its JVM tests pass (56), and CI builds and tests it, but the approval prompt and the lab screens have not been run on a device yet.
- A `read icons` contact sheet and clipboard commands: `eN` already names every unlabeled control, and Android 10+ does not let a background service read the clipboard reliably.
- The emulator chaos scenarios E1 to E8 from the test plan, and real-model runs of the 1.4 task file.

## 1.3.0 — 2026-10-24

**Highlights.** 1.3.0 hardens and speeds up 1.2 without changing its commands, tools, or flags. The helper now proves who it is before `au` talks to it, `au` installs only a helper signed with the pinned key, and the release path is tag-gated, signed in isolation, published to npm with provenance, and smoke-tested from the real packages on three systems. Agents act first, `set` is idempotent, misses suggest a fix, and a locked phone, a dark screen, or a killed helper each return a typed result with one next step. Every finding from the 1.2 review is fixed here, left to a named 2.0 component, or closed as won't-fix.

### Behavior changes

- `au uninstall` flags now work as named: `--dry-run` only reports, unknown flags are errors, and uninstall refuses while `au serve` is running.
- Host state is private and artifacts expire after 7 days; a shared `AU_HOME` on CI must become per user.
- au 1.3 upgrades the helper to 1.3 by itself, and going back to 1.2 needs `au uninstall` first.
- A label the agent never saw no longer runs if it is risky.
- `run notification action` needs a label when a notification has several actions.
- Newlines in screen text show as spaces.
- `toggle` checks that the switch flipped and fails when it did not.

### Security

- The helper now proves itself before `au` sends anything. The helper keeps a random per-process key that only the ADB shell user can read; `au` sends a random challenge on both helper sockets and continues only if each answer is the right HMAC-SHA256, so an app that binds the helper socket first can no longer impersonate the helper. A wrong or missing answer fails with "the process on the helper socket is not the Android-Use helper".
- The helper retries a socket bind that fails, tells you in a notification if it still cannot listen, and keeps accepting connections when one connection fails.
- Loose matches are safe near misses. A notification, tab, or app whose name only contains or starts with your words is used only when it is the single such item, lines up with your words at a word boundary, adds at most two other words, and none is risky (`pay`, `delete`, `transfer`, and forms like `payments`, `sending`, or `delete-all`); otherwise the command reports a miss that lists the candidates. `open app "Bank"` no longer opens "Bank Transfer".
- Device-side captures expire after 24 hours. Password fields are also recognized by their input type (text, visible, web, and number passwords), so their text is redacted wherever `isPassword()` was not set. Setup asks for notification permission (Android 13+) for the helper's own foreground notification.
- Debug and example Android builds never use the release signing key; the helper's release build uses it only when `AUBRIDGE_SIGNING_PROPERTIES` names a signing file.
- A release `au` installs only its embedded helper or an APK whose signer certificate matches the pinned release key (`device/release-signer.sha256`), checked on the host before `adb install`. It never passes `-d` (downgrade), and passes `-t` only in dev builds or with `AU_DEV_APK` set.
- Every cargo call in the workflows, `cargo xtask`, and the contributor docs uses `--locked`. `cargo-audit` and `cargo-deny` are installed at pinned versions, `deny.toml` rejects yanked crates, runners are `ubuntu-24.04`/`windows-2022` rather than `-latest`, and CI runs on pushes to `main` and on pull requests only.
- `install.sh` and `install.ps1` verify what they install: `ANDROID_USE_VERSION` must be a plain `X.Y.Z`, downloads are https with TLS 1.2+ only, the archive's build attestation is checked with `gh` when present (required with `AU_REQUIRE_ATTESTATION=1`) on top of `SHA256SUMS`, installs are one atomic move, and PowerShell finds the latest release through the `releases/latest` redirect instead of the unauthenticated API. Fixes #4, #40.
- New `cargo xtask workflow-policy`, run by `check` and `verify`: workflows must pin every action to a commit SHA, keep top-level permissions read-only, avoid `pull_request_target`, limit push triggers to `main` and `v*` tags, never pipe a download into a shell, and keep `persist-credentials: false` on every checkout. It is a text-level tripwire, not a YAML parser. Gradle dependencies are not yet locked.
- The release workflow is tag-gated: a first job checks the tag's commit is on `main` with CI green; the helper is built unsigned; signing runs in an isolated `release-signing` job with no checkout or repository code, and verification requires exactly one signer whose certificate matches `device/release-signer.sha256`; publishing runs in the `release` environment. A manual run builds everything and stops before signing. `workflow-policy` pins these properties. The `sign` job also refuses to run until `RELEASE_SIGNING_READY` is set on its environment. The owner-side settings are in [releasing](docs/contributing/release.md).

### Changed

- Agents act first: the instructions, `AGENTS.md`, and the skill no longer ask for a first read of the screen. In a served session, a label the agent was never shown runs only on an exact, unique, non-risky match inside the app it just opened; anything else is a miss that shows the screen. A label the agent never saw no longer runs if it is risky. (`au act` from a shell has no session and is unchanged.)
- A missing label now suggests up to three near labels from the screen and one `fix:` command to send as is (never with a risky label), and an ambiguous label says what each `number N` is.
- Lenient targets: a row copied from `screen` (`sw Wi-Fi=on`, `Save#2`) names its control when its text is not itself a label, curly quotes work as straight ones, and `click`, `turn on|off "LABEL"` (the same as `set "LABEL" on|off`) and `go back` are accepted. Every 1.x command parses as before; `computer/tests/command_corpus.rs` checks that against the commands in the docs, recorded from 1.2.0.
- `set "X" on|off` is new: it does nothing when the switch is already in that state, and otherwise taps and checks the state changed. `toggle` checks that it flipped and says so ("Wi-Fi is now on"), failing when it did not.
- Accessibility events no longer take the helper's UI lock (they bump an atomic generation and the last-event time, and a stale scene is rebuilt on demand), and `notificationTimeout` is 0 instead of 50. The quiet window after an action stays 200 ms but can be set with `AU_QUIET_MS`; `BENCH_QUIET_MS=200,150,120 cargo xtask benchmark-live` compares values.
- After an action, the screen is reported as soon as it is ready: the host polls the helper's screen generation every 40 ms instead of sleeping 500 ms up to three times while a screen is empty or "Loading", and a tap that left the generation unchanged waits up to 600 ms for a late change before reporting.
- Newlines in screen text show as spaces, and zero-width and other invisible format characters are dropped. Tabs and bidi override/isolate characters (U+202A–U+202E, U+2066–U+2069, U+200E, U+200F) do too, so screen text cannot fake extra rows or reorder text.
- `transfer` joins the risky words.
- `run notification action` needs a label when a notification has several actions: `run notification action "Reply" on "Messages"`. With several actions and no label nothing runs and the reply lists the action labels; a notification with one action still runs without a label. Notifications already showing when the listener connects are now listed, not only later ones.
- au 1.3 upgrades the helper to 1.3 by itself on connect (it reads the installed helper's `versionCode`); going back to 1.2 needs `au uninstall` first.
- The helper's `versionCode` is computed from `VERSION` (1.3.0 is 1,003,000), and `cargo xtask version` checks it.
- Host state is private and artifacts expire after 7 days; a shared AU_HOME on CI must become per user. On macOS and Linux the state folder, `device.json`, journal, locks, logs, artifacts, and `screen-space.json` are created 0700/0600, existing ones owned by you are tightened at startup, and artifacts older than 7 days are deleted.
- The npm launcher fails closed: `android-use` and each `@androiduse/<platform>` package ship a `manifest.json` with SHA-256 digests, and a missing manifest, a missing entry for your platform, or a checksum mismatch exits 1 instead of running an unchecked binary. `AU_BIN` still overrides the binary but warns on stderr every time. The launcher forwards SIGINT, SIGTERM, and SIGHUP to `au` and exits with its code. A binary swapped between the check and the launch is not defended against: anyone who can write `node_modules` can rewrite the launcher itself.
- `au setup` run through `npx` registers `npx -y android-use@X.Y.Z serve --mcp` (pinned to its own version) with agent clients instead of a path inside the npx cache, which disappears when the cache is pruned.
- The MCP config and install docs pin the release an agent runs (`npx -y android-use@X.Y.Z`, and the `vX.Y.Z` install URLs) instead of `android-use@1` or `main`; `cargo xtask version` fails when a pin disagrees with `VERSION`, so a broken or compromised `latest` cannot change what an agent executes.
- Acting is 10 to 40 ms faster: the journal is reloaded only when another process wrote it.
- npm publishing moves into CI with npm trusted publishing (OIDC) and `--provenance`: a new `npm` job in the release workflow, in the `npm` environment and the only job with `id-token: write` besides attestation, publishes `android-use` and the six `@androiduse/<platform>` packages with no npm token. `install/publish-npm.mjs` refuses to run outside GitHub Actions unless `--dry-run`, and runs `gh attestation verify` on every release asset before staging; any failure stops it. `install/render-formula.mjs` renders the Homebrew formula only from an attestation-verified `SHA256SUMS` (it verifies one it downloads, and refuses a file without `--verified`). `workflow-policy` pins where `id-token: write` and npm publishing may appear. The owner-side registration is in [releasing](docs/contributing/release.md).

### Added

- `cargo xtask bench-model` (contributor tooling): drives a real agent CLI through fifteen device goals, verifies success from device state, and compares two `au` builds with calls, input tokens, and wall time as means with 95% confidence intervals. It has a spend guard (`--max-runs`, `--max-cost`).
- Device preflight: before the first action of a session and after any failure, Android Use checks the phone and returns a typed `locked`, `screen_off`, or `helper_killed` result with one next step, instead of a timeout or a permission error. The helper has a new `env` operation. `au status` and `au doctor` report when battery optimization is on for Android-Use or TalkBack is enabled. `cargo xtask emulator-e2e` covers the three results.
- The Android-Use app shows each grant (Accessibility, notification access, notification permission, battery exemption), the helper version, and the last contact from the computer. The foreground notification has a **Stop** action; afterwards `act` returns a typed `stopped` result until `au setup` runs. The launcher and notification icons are vector drawables.
- Helper `tree` op (status capability bit 3) that dumps every accessibility window and its nodes, capped at 2,048 nodes and 900 KB with a truncation flag, password text omitted, with a typed Rust reader and a shared golden fixture. For tools and 2.0; no agent command uses it.

### Fixed

- Camera, microphone, and location capture on Android 14+ (#5, #21, #22, #24): the helper declares and claims the matching foreground-service types, a microphone that returns only zeros is an error instead of a silent recording, a camera that opens after the capture timed out is closed at once, a location read always completes (including when a provider is disabled), and captures no longer hold the UI lock, so taps and reads keep working during them. Overlapping captures keep each other's service types, so a location read cannot stop a screen recording.
- Screen recording works on Android 14+ (#7): the helper promotes its foreground service to the `mediaProjection` type first, then gets the projection, registers its callback, and only then creates the virtual display. Android 14 allows a screen-capture grant once, so the helper clears it after use and a second recording asks again instead of crashing.
- `au setup`, `au install`, and `au uninstall AGENT` write agent config safely: only a missing file counts as empty (any other read error aborts without writing), a failed backup aborts the write, the file's existing mode is kept, `claude mcp` and `codex mcp` are killed after 30 seconds, and a failed re-add restores the old entry.
- `au uninstall` flags now work as named: `--dry-run` only reports, unknown flags are errors, and uninstall refuses while `au serve` is running.
- `cargo xtask` gates fail closed: a missing release artifact fails the size gate instead of measuring 0 bytes, and the stale-version check flags only standalone versions such as `v2` or `v3.1`, not words that contain one (`dev2`).
- `au serve --mcp` answers a bad frame with a JSON-RPC error and keeps serving: an oversized line (`-32600`, its rest discarded), invalid JSON (`-32700`), and a batch array (`-32600`, batches are not supported) used to end the server or get a non-JSON-RPC reply. An oversized operation journal is saved aside once, with a warning, and a fresh one keeps its pending records and newest receipts; owner sidecar files left by exited processes are removed.
- `cargo xtask golden-capture NAME` needs `--reviewed`, confirming the screen shows nothing personal, and refuses a screen with what looks like an email address, phone number, or card number.
- `verify eN` checks that handle instead of searching for the label "eN"; `wait for eN`, `verify eN is gone`, and `verify "X" number N is gone` are rejected instead of silently dropping the handle or number; a reconnect to the helper clears the cached screen; an `eN` from an earlier screen is refused with "read the screen again".
- Every wait now has a deadline: a hung `adb`, `emulator`, or `sdkmanager` child or a stalled output pipe is stopped with a clear error, each Chrome DevTools command times out instead of waiting forever, `au emulator snapshot` and `destroy` touch only emulators `au emulator create` made (a foreign AVD or emulator is refused), and a failed `au emulator start` kills the emulator it spawned.

### Maintenance

- New `package-smoke` CI job on Linux, macOS and Windows: it packs the real npm packages, installs them into a fresh prefix, runs `read help` and an MCP `initialize` through the launcher, and proves a tampered binary and a missing manifest both exit 1. It found that `@androiduse/<platform>` was published without its `manifest.json`, so the fail-closed launcher would have refused every npm install; the package now ships it.
- Budgets (formatted lines), raised once for 1.3.0: Rust production 9,100, Java 4,700, automation 3,200, authored code 22,800. They were full (Rust 7,489 of 7,520, authored 15,512 of 15,550). The new caps are what the 33 other 1.3.0 PRs measure together (Rust 8,689, Java 4,382, automation 2,923, authored 21,358) plus about 5% for review fixes. The largest additions are the all-windows tree op (Java +361), the helper handshake (Java +225), the real-model benchmark (automation +355) and the workflow policy check (automation +187).
- Screen presentation (the text view, `Changed:` diffs, and screen images for the model) moved out of `agent.rs` into `present.rs`; no behavior change.
- `DeviceLink` is now the one seam between the engine and a device: `AdbLink` is the real adb and bridge path, and a scripted in-process `FakeLink` replaces the socket-based test helper. No behavior or wire change.
- The host binary size limits rise once for 1.3.0, to 1,120,000 bytes (1,450,000 on Linux) from 1,000,000 (1,300,000): the 1.3.0 Linux release binary is 1,328,904 bytes, with the device link, helper handshake, APK signature check, preflight, and policy work. The helper APK limit stays 60,000 (48,168).

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
- The helper waits for the screen to react and go quiet inside the plan, so the host polls nothing. The helper link and Chrome use the adb server's own protocol: no `adb` process per command (only to start a stopped adb server), no port forward. A cold `au read status` fell from 150 ms to 70 ms. Artifacts download in 256 KiB slices (they were 2.8 KB).

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
