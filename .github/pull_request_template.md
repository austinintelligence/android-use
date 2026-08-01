## What changed

<!-- Describe the user-visible contract and why it is needed. -->

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
- [ ] `npm test --workspace packages/installer`
- [ ] `npm run docs:check`
- [ ] `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check-public.ps1`

Live-device evidence is required for changes to transport, helper, media,
location, web, or cleanup behavior. Redact serials and private artifacts.
