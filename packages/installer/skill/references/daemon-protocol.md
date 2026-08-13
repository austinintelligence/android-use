# Daemon and fast path

The first fast command starts a hidden per-user Rust daemon. Windows uses `\\.\pipe\codex-android-use-v1` with an owner-only DACL. macOS and Linux use `state/daemon.sock` with mode `0600` inside a mode-`0700` state directory. Both carry bounded length-prefixed native `AU2` frames. Native request/response frames carry the protocol version, request ID, typed operation, exact UTF-8 argument boundaries, and JSON only for the structured result value. The daemon accepts one compatibility JSON frame mode for the benchmark harness; new clients use native frames. A protocol-version handshake, bounded frame sizes, and structured protocol errors are required. A daemon state record is trusted only after PID, executable path, binary version, protocol version, current-user ownership/private socket, and handshake validation.

One connected local-IPC stream accepts multiple sequential frames. One-shot
commands may close after their response; native clients should keep the
connection open to amortize pipe creation and carry a batch of requests. The
foreground `au pipe` mode is line-streaming: each non-empty DSL line is
executed against one warm shell/helper/CDP context and its compact response is
emitted before the next line is read. A malformed or truncated frame is not
resynchronized: the daemon returns a bounded protocol error when possible,
discards only that connection, and accepts a fresh one.

The daemon maintains a framed `adb -s SERIAL shell` per active endpoint. Each transaction has a random marker and explicit exit status. It is discarded on timeout, endpoint loss, broken pipe, desynchronization, invalid framing, or unexpected exit. A cached automatic endpoint is discarded with its failed shell, so rediscovery must re-check exact hardware identity.

The daemon also keeps a validated endpoint-selection cache for adjacent
semantic, web, app, system, and media calls. A cached endpoint is reusable only
when its recorded `ro.serialno` equals the configured hardware serial and it
still satisfies the requested selector (`usb`, `wifi`, `mdns`, or an exact
endpoint). Pairing, connection, disconnect, identity, shell, and ADB errors
invalidate the cache. This removes repeated `adb devices -l` and serial probes
without weakening failover.

Use `au daemon status` for compact handshake proof, `au daemon stop` for an orderly stop, or `au pipe` when a foreground persistent DSL session is preferable.
