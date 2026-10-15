# Security

Android Use can observe and operate a physical device. Treat access to `au`, its MCP process, the host account, and Android USB debugging as device-control authority.

## Supported versions

Security fixes are applied to the current `1.x` release line. Please upgrade to the latest `1.x` release before reporting an issue unless doing so would make the problem impossible to reproduce.

## Report a vulnerability privately

Use [GitHub Security Advisories](https://github.com/austinintelligence/android-use/security/advisories/new) for private reports. If private reporting is unavailable, open a minimal public issue asking for a private contact channel; do not include exploit details there.

A useful report includes the Android Use version, host OS, Android version, device transport, the smallest safe reproduction, expected and actual behavior, and any mitigation you tested. Remove device serials, helper credentials, screenshots, recordings, notification content, account data, and browser content.

We will acknowledge reports, assess impact, and coordinate a fix or mitigation as capacity permits. Do not publish exploit details until a fix is available or we agree on disclosure timing. We do not promise a specific response or remediation time.

## Scope

In scope: the `au` CLI, the Android helper, release archives, installer integrity verification, and repository-maintained GitHub Actions. Third-party Android builds, a host already compromised by malware, an unauthenticated network wrapper around `au serve`, and user-approved actions on a device are outside the product's direct security boundary, but reports that clarify a boundary are still welcome.

## Trust boundaries

- **Local host:** `au serve` uses stdio. It does not open a TCP listener. If another system carries stdio over a network, that system must provide authentication, encryption, and authorization.
- **ADB transport:** Android must authorize the computer's debugging key. The server then binds one enrolled hardware serial to the session. Wireless ADB, if configured outside Android Use, expands this trust boundary to that network.
- **Android helper:** the service has no `INTERNET` permission. It accepts an abstract-local socket from Android shell UID 2000. `au` reaches it by speaking the ADB server's own protocol (`host:transport:SERIAL`, then `localabstract:NAME`), so no `adb` process runs per command (`adb` is run only to start a stopped adb server) and no TCP port forward exists to leak.
- **Helper identity:** before `au` sends any request, the helper must answer a random challenge with HMAC-SHA256 under a per-process key, on both the bootstrap socket and the command socket (the second answer is bound to the session token). The key is generated when the helper starts and is readable only by the ADB shell user (a content provider that checks the caller's uid, with the `DUMP` permission as a second gate), so another app that binds the helper's socket first cannot produce either answer and `au` drops the connection before it sends a plan. `au` reads the key once per device and caches it until an answer fails. This is a new security claim in 1.3.0. It does not defend against code running as the shell user or root.
- **Chrome:** CDP access is loopback-only and allowlisted to Chrome's Android abstract socket. It can read and operate the selected Chrome session, including authenticated pages already open on the device.

Report vulnerabilities privately through GitHub Security Advisories. Do not include device identifiers, bridge credentials, UI content, or captured artifacts in a public issue.

The normal product binds one enrolled hardware serial to a currently connected ADB transport. The Android service is startable only by the privileged `DUMP` caller, accepts abstract-local connections only from shell UID 2000, issues fresh bootstrap credentials, authenticates once per command connection, and requires exact monotonically increasing request sequences. It has no `INTERNET` permission.

All frames, queues, workers, plans, strings, predicates, waits, mutations, scenes, artifacts, and inline responses are bounded. The helper parses and validates the complete plan before mutation, checks the expected UI generation immediately before the first mutation, executes only forward, and stops on the first failure. It keeps the last 128 final receipts until it restarts. The host durably records a pending digest before sending. If a process ends with an operation still pending, every new action is refused as `unknown` until the agent reads the screen, which settles the old outcome as unknown, so an uncertain action is never silently repeated. Operation ids for agent commands are salted per process, so a restarted client reusing request ids cannot replay an older record.

The production Rust crate forbids unsafe code and exposes no model-accessible ADB or shell escape hatch. Screenshots and photos an agent explicitly requests are returned to the connected client as image content; audio, screen recordings, and expanded diagnostics stay in the private artifact folder and only their path or handle is returned. Credentials, device identity, private content, and Java exception details are not logged.

## Agent configuration and distribution

`au install` edits agent configuration files (for example `~/.cursor/mcp.json`, `~/.claude.json` through `claude mcp add`, and `~/.codex/config.toml`). It changes only the `android-use` entry, saves the previous file as `<file>.au-backup` (if the backup fails, nothing is written), keeps the file's mode, treats only a missing file as empty (any other read error stops the edit), kills an agent CLI it runs (`claude mcp`, `codex mcp`) after 30 seconds, restores the old entry if re-adding fails, refuses to overwrite an `android-use` entry it did not write unless you pass `--force`, and does not edit files that contain comments. `au install --dry-run` shows the change first. A tool an agent registers runs with that agent's permissions, so register it only with agents you trust to operate your device.

Releases are built by GitHub Actions from a tagged commit and published with `SHA256SUMS`, an SPDX SBOM, and build attestations; the helper APK is signed with a pinned key whose fingerprint is checked in CI. Debug and example builds never use that key: only the helper's release build type signs with it, and only when `AUBRIDGE_SIGNING_PROPERTIES` names the signing file. A release `au` installs only its embedded helper, or an APK whose v2/v3 signer certificate has the SHA-256 pinned in `device/release-signer.sha256`; it checks this on the host before `adb install` (Android verifies the signature itself), never downgrades, and passes `-t` only in dev builds or when `AU_DEV_APK` is set. The install scripts verify `SHA256SUMS` before unpacking. The npm packages (`android-use` and `@androiduse/<platform>`) and the Homebrew formula carry the same binaries, and the launcher checks the binary against its manifest before running it.

## Emulator targets

`au emulator` creates local emulators, each a separate target with its own enrollment, journal, artifacts, and an exclusive `au serve` lock. An emulator is enrolled by a random per-target identifier stored in its settings, never by port, so a different device that later takes the same port is refused. Android Use does not enable the Accessibility service on an emulator or a phone; the user does, through Android's own consent screen. Emulators bind to loopback only. See [emulators](docs/guides/emulators.md).

## Sensitive capabilities

Camera and microphone plans are permission-gated and bounded. Location is a one-shot request. Notification access requires Android's user-enabled listener service. Screen recording requires a process-local user-granted MediaProjection token and is capped at 30 seconds. Screenshots can contain credentials, messages, and personal data even though they do not require the camera permission. Browser CDP can access authenticated page content. Agents cannot submit arbitrary page JavaScript; Android Use exposes only the documented browser operations.

Grant optional Android permissions only for the task that needs them. Require human approval before capture, purchases, submissions, deletion, account changes, or acting on sensitive notifications. USB debugging and wireless debugging trust the host as a device-control peer; revoke that trust in Android Developer options when you no longer need Android Use.

## Data and persistence

On Windows, state is stored under `%LOCALAPPDATA%\AndroidUse`; on macOS and Linux under `$XDG_STATE_HOME/android-use`, defaulting to `~/.local/state/android-use`. `AU_HOME` overrides both. It includes the enrolled device identity, an operation journal, and private artifacts. The journal stores bounded operation metadata and digests, not media payloads or raw UI dumps. On macOS and Linux folders are mode 0700 and files 0600, created that way, and existing ones owned by you are tightened at startup (anything owned by someone else is left alone with a warning). Artifacts (captures, recordings, downloads) older than 7 days are deleted when a target opens. Artifact contents remain local unless a connected client fetches and transmits them.

`au uninstall` removes the helper from the enrolled device and deletes Android Use's local enrolled-device file, journal, and artifact directory. It does not remove ADB itself or unrelated device data. `au uninstall --dry-run` lists these without changing anything, and uninstall refuses while `au serve` holds the state lock.

Do not publish diagnostics without reviewing them. Do not place Android Use state or artifacts in a synchronized or world-readable directory. Use host disk encryption where captures may be sensitive.

## Authentication and remote use

The helper issues fresh bootstrap credentials, authenticates once per command connection, and requires exact monotonically increasing request sequences. This protects the helper socket; it is not a user-account system for a remotely exposed MCP server. Android Use does not ship a remote broker or multi-tenant authorization layer; a design for one is in [docs/design/remote-broker.md](docs/design/remote-broker.md), and nothing in it is built.

Run `cargo xtask verify` for Rust and Java tests, the shared cross-language golden vector, source budgets, release builds, version consistency, documentation gates, and binary/APK size output.
