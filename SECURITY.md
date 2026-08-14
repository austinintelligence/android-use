# Security

Android Use can observe and operate a physical device. Treat access to `au`, its MCP/JSONL process, the host account, and Android USB debugging as device-control authority.

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
- **Android helper:** the service has no `INTERNET` permission. It accepts an abstract-local socket from Android shell UID 2000 and is reached through an AU-created ADB forward.
- **Chrome:** CDP access is loopback-only and allowlisted to Chrome's Android abstract socket. It can read and operate the selected Chrome session, including authenticated pages already open on the device.

Report vulnerabilities privately through GitHub Security Advisories. Do not include device identifiers, bridge credentials, UI content, or captured artifacts in a public issue.

The normal product binds one enrolled hardware serial to a currently connected ADB transport. The Android service is startable only by the privileged `DUMP` caller, accepts abstract-local connections only from shell UID 2000, issues fresh bootstrap credentials, authenticates once per command connection, and requires exact monotonically increasing request sequences. It has no `INTERNET` permission.

All frames, queues, workers, plans, strings, predicates, waits, mutations, scenes, artifacts, and inline responses are bounded. The helper parses and validates the complete plan before mutation, checks the expected UI generation immediately before the first mutation, executes only forward, and stops on the first failure. It caches final operation IDs for its lifecycle. The host fsyncs a pending digest before sending; an interrupted pending operation becomes `unknown` and is never automatically repeated.

The production Rust crate forbids unsafe code and exposes no model-accessible ADB or shell escape hatch. Media and expanded diagnostics return private artifact handles instead of transcript payloads. Credentials, device identity, private content, and Java exception details are not logged.

## Sensitive capabilities

Camera and microphone plans are permission-gated and bounded. Location is a one-shot request. Notification access requires Android's user-enabled listener service. Screen recording requires a process-local user-granted MediaProjection token and is capped at 30 seconds. Screenshots can contain credentials, messages, and personal data even though they do not require the camera permission. Browser CDP can access authenticated page content. Agents cannot submit arbitrary page JavaScript; Android Use exposes only the documented browser operations.

Grant optional Android permissions only for the task that needs them. Require human approval before capture, purchases, submissions, deletion, account changes, or acting on sensitive notifications. USB debugging and wireless debugging trust the host as a device-control peer; revoke that trust in Android Developer options when you no longer need Android Use.

## Data and persistence

On Windows, state is stored under `%LOCALAPPDATA%\AndroidUse`; on macOS and Linux it uses the platform local data directory. It includes the enrolled device identity, an operation journal, and private artifacts. The journal stores bounded operation metadata and digests, not media payloads or raw UI dumps. Artifact contents remain local unless a connected client fetches and transmits them.

`au uninstall` removes the helper from the enrolled device and deletes Android Use's local enrolled-device file, journal, and artifact directory. It does not remove ADB itself or unrelated device data.

Do not publish diagnostics without reviewing them. Do not place Android Use state or artifacts in a synchronized or world-readable directory. Use host disk encryption where captures may be sensitive.

## Authentication and remote use

The helper issues fresh bootstrap credentials, authenticates once per command connection, and requires exact monotonically increasing request sequences. This protects the helper socket; it is not a user-account system for remotely exposed MCP or JSONL. Android Use does not ship a remote broker or multi-tenant authorization layer.

Run `cargo xtask verify` for Rust and Java tests, the shared cross-language golden vector, source budgets, release builds, version consistency, documentation gates, and binary/APK size output.
