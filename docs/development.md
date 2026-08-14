# Development

The host is Rust. The helper is a small Android Java application.

## Build

```console
cargo build --release --bin au
cargo xtask package
```

Android helper builds require the Android SDK and the Gradle wrapper configured under `device/`.

## Verify

```console
cargo xtask verify
npm test
```

Live checks use one authorized device:

```console
cargo xtask live
cargo xtask stress-live
```

Privacy-sensitive capture checks require explicit Android permission. Do not enable them merely to satisfy an unrelated test.

## Source map

- `computer/` — engine, adapters, protocol, artifacts, browser, and installer lifecycle.
- `device/` — authenticated Android helper and deterministic debug example.
- `tools/` — verification, packaging, live tests, and benchmarks.
- `install/` — lightweight npm launcher for bundled platform releases.
- `skills/` — compact operational instructions for coding agents.

Read [CONTRIBUTING.md](../CONTRIBUTING.md) and [SECURITY.md](../SECURITY.md) before changing trust boundaries.
