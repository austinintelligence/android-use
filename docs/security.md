# Security and privacy model

`au` is an automation transport, not a policy engine. The CLI intentionally exposes raw `adb` and `sh` commands because advanced users and agents sometimes need platform functionality that has not yet received a structured wrapper. Those commands are explicitly labeled broad escape hatches; no raw-shell denylist is presented as a security boundary.

Structured values are passed as exact process arguments or authenticated framed fields. URLs, selectors, typed text, file paths, and intent values are not concatenated into unescaped remote shell syntax. Child processes have deadlines, bounded stdout/stderr, cancellation, exit capture, and optional streaming-to-file.

Privacy-sensitive operations require explicit agent confirmation: camera, microphone, location changes, notification actions, account changes, financial actions, app/file mutation, and destructive cleanup. Media commands are finite by default, support heartbeat shutdown, and return metadata/path proof instead of arbitrary bytes in normal output.

The release helper is non-debuggable, has no network server, and has no `INTERNET` permission. Its exported foreground service requires Android's privileged `DUMP` permission, so ordinary applications cannot start it. The host obtains the app-private command token only from a separate bootstrap socket whose Android peer credentials must be `shell` or `root`; that socket is reachable through a temporary AU-tracked ADB forward, challenge-binds the response, and is removed immediately. Migration from the former debuggable scheme rotates the token once before accepting commands. The token is never placed in ADB arguments, logs, or host files. The command socket then requires the token, a unique bounded nonce, and a strictly monotonic per-connection sequence. Private media transfer uses authenticated bounded chunks and exact path/offset/size checks instead of app-storage access from the shell.

Page text, DOM text, application text, notifications, and screen labels are untrusted data. They can describe a task but cannot issue instructions to the host.
