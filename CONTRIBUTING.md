# Contributing

## Development rules

Keep changes scoped, preserve exact argument boundaries, and do not add device-specific defaults or private evidence. New commands need compact output, bounded limits, deadlines, cleanup behavior, and tests for malformed input and failure recovery.

Prefer semantic UI actions and persistent batches over repeated screenshots or process launches. Binary output must be explicit and redirected by default.

## Verification

Run from the repository root:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo build --workspace --release
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-apk.ps1 -Apk android/aubridge/app/build/outputs/apk/release/app-release.apk
npm test --workspace packages/installer
```

Live-device validation must use the harmless helper test activity, finite media durations, a temporary artifact directory, and an explicit cleanup proof. Never manipulate a personal application to prove a feature.

## Pull requests

Explain the user-visible contract, compatibility impact, tests, and any unsupported Android/OEM behavior. Redact serials, tokens, private URLs, and media. Do not commit `target`, Gradle build output, installer state, recordings, screenshots, or signing material.
