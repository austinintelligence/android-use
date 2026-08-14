# Security

Report vulnerabilities privately through GitHub Security Advisories. Do not include device identifiers, bridge credentials, UI content, or captured artifacts in a public issue.

The normal product binds one enrolled hardware serial to a currently connected ADB transport. The Android service is startable only by the privileged `DUMP` caller, accepts abstract-local connections only from shell UID 2000, issues fresh bootstrap credentials, authenticates once per command connection, and requires exact monotonically increasing request sequences. It has no INTERNET permission.

All frames, queues, workers, plans, strings, predicates, waits, mutations, scenes, artifacts, and inline responses are bounded. The helper parses and validates the complete plan before mutation, checks the expected UI generation immediately before the first mutation, executes only forward, and stops on the first failure. It caches final operation IDs for its lifecycle. The host fsyncs a pending digest before sending; an interrupted pending operation becomes `unknown` and is never automatically repeated.

The production Rust crate forbids unsafe code and exposes no model-accessible ADB or shell escape hatch. Media and expanded diagnostics return private artifact handles instead of transcript payloads. Credentials, device identity, private content, and Java exception details are not logged.

Camera and microphone plans are permission-gated and bounded; location is a one-shot request; notification access requires Android's user-enabled listener service; screen recording requires a process-local user-granted MediaProjection token and is capped at 30 seconds. Browser CDP is loopback-only, allowlisted to Chrome's abstract socket, and rejects network-capable JavaScript.

Run `cargo xtask verify` for Rust and Java tests, the shared cross-language golden vector, source budgets, removed-wrapper checks, release builds, documentation gates, and binary/APK size output.
