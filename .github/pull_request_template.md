## What changed

<!-- Describe the user-visible contract and why it is needed. -->

## Why

<!-- What problem does this solve? -->

## Safety and compatibility

- [ ] No device-specific serials, tokens, private paths, media, or signing material are included.
- [ ] No new way for an agent to run a shell, `adb`, or page JavaScript; command forms changed in `computer/src/spec.rs` only.
- [ ] Destructive/privacy-sensitive behavior has explicit confirmation coverage.
- [ ] Temporary files, processes, emulators, and device state have cleanup coverage.

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --all-targets`
- [ ] `cargo xtask verify`
- [ ] `npm test` and `npm run lint`

## Release impact

- [ ] No breaking change.
- [ ] Breaking change documented in the changelog and migration guidance.

Live-device evidence is required for changes to transport, helper, media,
location, web, or cleanup behavior. Redact serials and private artifacts.
