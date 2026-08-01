# Architecture

## Runtime layers

1. `au` parses commands, applies bounded limits, selects an endpoint, and emits compact proof.
2. The per-user daemon owns warm ADB transports and the Windows named pipe. It is replaceable only after PID, path, binary version, protocol version, and handshake validation.
3. The ADB layer preserves exact argv boundaries, uses bounded child execution, and tracks only AU-created forwards/reverses.
4. The helper uses an authenticated abstract local socket reached through an AU-owned ADB forward. Accessibility, notifications, Camera2, AudioRecord, and Android test-provider work remain inside the helper.
5. Chrome control uses a temporary CDP forward and treats DOM/page text as untrusted input.

## Fast paths

- Cold CLI: one Rust process and one bounded action.
- Daemon: named-pipe handshake followed by one framed request.
- Shell batch: one persistent interactive ADB shell transaction for compatible actions.
- Semantic batch: one helper session with session-scoped handles; semantic boundaries are never incorrectly lowered into shell text.
- `au pipe`: foreground JSONL/DSL mode for clients that want one long-lived process without daemon IPC per action.

## Identity model

The initial configuration is unenrolled. An explicit `au u ENDPOINT` call reads and stores the endpoint's `ro.serialno`. Discovery groups USB, IP, and mDNS endpoints by that exact value. Model, product, alias, and remembered endpoint names never establish identity.

## State ownership

Persistent state belongs under `%LOCALAPPDATA%\Codex\android-use`: canonical JSON configuration, migration markers, daemon metadata, helper-forward registry, location restoration journal, release versions, and user artifacts. Configuration writes are serialized with `serde_json`, written to a temporary file, flushed, and atomically replaced with a recoverable previous copy where applicable.
