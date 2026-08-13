# Architecture

## Runtime layers

1. `au` parses commands, applies bounded limits, selects an endpoint, and emits compact proof.
2. The per-user daemon owns warm ADB transports. Windows uses a current-user ACL named pipe; macOS/Linux use a mode-`0600` Unix socket under a mode-`0700` state directory. A daemon is replaceable only after PID, executable, binary version, protocol version, and handshake validation.
3. The ADB layer preserves exact argv boundaries, uses bounded child execution, and tracks only AU-created forwards/reverses. On the standard loopback ADB server it uses the small classic host protocol directly for read-only `devices-l` and `get-state` queries; any framing, timeout, server, or non-loopback condition falls back to the official `adb` client. Shell, install, pairing, forwarding, and lifecycle operations always remain with official platform-tools.
4. The non-debuggable helper uses a shell-UID-gated credential-bootstrap socket plus an authenticated command socket, each reached only through temporary/AU-owned ADB forwards. Accessibility, notifications, Camera2, AudioRecord, private chunked artifact transfer, and Android test-provider work remain inside the helper.
5. Chrome control uses a temporary CDP forward and treats DOM/page text as untrusted input.

## V2 agent boundary

```text
agent adapter (MCP / JSONL / Codex / portable)
        |
        +--> android.status / observe / execute / artifact / recipe
                         |
                  bounded contract runtime
                         |
       observation/capability cache + semantic plan compiler
                         |
             ADB-backed transport (USB / Wi-Fi / mDNS)
                         |
              AU Bridge (local, authenticated, no INTERNET)
```

One `ContractRuntime` lives for the entire JSONL or MCP session and owns warm
device selection, persistent shell, helper, and browser pools. It caches safe
capability data for two seconds while checking reachability and exact hardware
identity live. The model receives stable references, generation checks,
bounded receipts, and typed failures rather than transport details.
Remote transport types are intentionally present as a policy boundary only;
the repository does not enable an Internet relay by installing the local
helper.

## Fast paths

- Cold CLI: one Rust process and one bounded action.
- Daemon: one authenticated local-IPC handshake followed by framed requests.
- Shell batch: one persistent interactive ADB shell transaction for compatible actions.
- Semantic batch: adjacent tap/text/scroll/wait/assert/back steps compile to one bounded device-resident `plan.run` frame (32 contract steps/16 mutations); semantic boundaries are never lowered into shell text. A smaller tap/wait/assert proof frame remains for that exact common shape.
- Dense observation: `[ref,label,role,flags]` tuples omit repetitive keys and geometry until requested.
- Receipt recovery: helper frames report a known completed prefix; transport loss remains an explicit unknown commit.
- Immutable artifacts: the helper opens a private artifact into a short-lived handle bound to path, size, modification time, and SHA-256; the host streams and re-hashes it before exact device cleanup.
- `au pipe`: foreground JSONL/DSL mode for clients that want one long-lived process without daemon IPC per action.

## Identity model

The initial configuration is unenrolled. An explicit `au u ENDPOINT` call reads and stores the endpoint's `ro.serialno`. Discovery groups USB, IP, and mDNS endpoints by that exact value. Model, product, alias, and remembered endpoint names never establish identity. A cached non-USB endpoint is also bound to the same live ADB transport ID; reconnecting with a new transport ID forces a fresh hardware-identity probe before helper or shell use.

## State ownership

Persistent state uses `%LOCALAPPDATA%\Codex\android-use` on Windows,
`~/Library/Application Support/android-use` on macOS, and
`${XDG_DATA_HOME:-~/.local/share}/android-use` on Linux. `AU_STATE_ROOT` is an
explicit portable override. These roots hold canonical configuration, setup
and agent journals, daemon metadata, helper-forward registry, operation
receipts, location restoration, release versions, and user artifacts. Windows
retains its v1 root to preserve enrolled identity. Configuration writes are serialized with
`serde_json`, written to a temporary file, flushed, and atomically replaced
with a recoverable previous copy where applicable.
