# Security policy

## Supported releases

Security fixes target the newest release line. Pin a release version in automation rather than using an unreviewed moving binary.

## Reporting

Do not publish device serials, ADB tokens, helper signing keys, screenshots, recordings, account data, or exploit details in a public issue. Use a private GitHub security advisory or contact the repository owner through the private channel listed on the repository profile.

Include the release version, host OS, helper version, reproduction command with secrets and personal identifiers removed, and the smallest relevant log excerpt.

## Security boundaries

- The helper has no network server and no `INTERNET` permission.
- Helper commands require token authentication, bounded frames, nonces, and per-session sequence ordering.
- The Windows daemon uses a current-user named pipe and versioned length-prefixed frames.
- Structured commands preserve argument boundaries; raw `adb` and `sh` are intentionally unrestricted escape hatches and are not a security boundary.
- Page and application text is untrusted data and must not become host instructions.
- Device failover requires exact `ro.serialno` identity equality.

These properties reduce accidental exposure; they do not authorize actions. Agents must still confirm destructive and privacy-sensitive operations.
