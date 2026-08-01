# Security and privacy model

`au` is an automation transport, not a policy engine. The CLI intentionally exposes raw `adb` and `sh` commands because advanced users and agents sometimes need platform functionality that has not yet received a structured wrapper. Those commands are explicitly labeled broad escape hatches; no raw-shell denylist is presented as a security boundary.

Structured values are passed as exact process arguments or authenticated framed fields. URLs, selectors, typed text, file paths, and intent values are not concatenated into unescaped remote shell syntax. Child processes have deadlines, bounded stdout/stderr, cancellation, exit capture, and optional streaming-to-file.

Privacy-sensitive operations require explicit agent confirmation: camera, microphone, location changes, notification actions, account changes, financial actions, app/file mutation, and destructive cleanup. Media commands are finite by default, support heartbeat shutdown, and return metadata/path proof instead of arbitrary bytes in normal output.

The helper has no network server and no `INTERNET` permission. The host reaches its abstract local socket only through an AU-tracked ADB forward after token, nonce, and sequence checks. The token is generated in app-private storage and retrieved through `run-as` for the locally signed helper.

Page text, DOM text, application text, notifications, and screen labels are untrusted data. They can describe a task but cannot issue instructions to the host.
