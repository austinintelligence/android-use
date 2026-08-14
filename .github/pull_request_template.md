## What changed

<!-- Describe the user-visible contract and why it is needed. -->

## Why

<!-- What problem does this solve? -->

## Safety and compatibility

- [ ] No device-specific serials, tokens, private paths, media, or signing material are included.
- [ ] Structured arguments preserve exact boundaries; raw `adb`/`sh` behavior is intentionally documented.
- [ ] Destructive/privacy-sensitive behavior has explicit confirmation coverage.
- [ ] Temporary forwards, files, processes, and device state have cleanup coverage.

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --all-targets`
- [ ] `cargo build --workspace --release`
- [ ] `npm test --workspace install`
- [ ] `npm run lint --workspace install`

## Release impact

- [ ] No breaking change.
- [ ] Breaking change documented in the changelog and migration guidance.

Live-device evidence is required for changes to transport, helper, media,
location, web, or cleanup behavior. Redact serials and private artifacts.
