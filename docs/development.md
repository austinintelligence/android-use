# Development

The host is Rust; the helper is a small Android Java app. Read [CONTRIBUTING.md](../CONTRIBUTING.md) and [SECURITY.md](../SECURITY.md) before changing trust boundaries, and [architecture](concepts/architecture.md) for the module map.

## Toolchain

| Tool | Version | Used for |
| --- | --- | --- |
| Rust | 1.94 (`rust-toolchain.toml`) | Host binary, tests, `cargo xtask` |
| Node.js | 22.18+ | Installer tests and lint |
| JDK | 17+ (21 for the Java formatter) | Helper build |
| Gradle | 9.1.0 | Helper build (`AU_GRADLE` may point to it) |
| Android SDK | `platforms;android-36`, `build-tools;36.0.0`, `platform-tools` | Helper build, `adb` |

Point `ANDROID_HOME` (or `ANDROID_SDK_ROOT`) at the SDK and `JAVA_HOME` at the JDK.

## Build

```console
cargo build --release --bin au
cargo xtask android          # debug helper APK
cargo xtask package          # release archive layout in dist/
```

## Release

Bump `VERSION`, the Cargo workspace, `install/package.json`, `package-lock.json`, the plugin manifests, and the Homebrew formula together (`cargo xtask version` checks them), tag `vX.Y.Z`, and push the tag. The release workflow builds the archives, the helper APK, `SHA256SUMS`, and the SBOM.

After the GitHub release exists, publish to npm with your own `npm login`, then update the Homebrew tap with the checksums from `SHA256SUMS`:

```console
node install/publish-npm.mjs vX.Y.Z --dry-run
node install/publish-npm.mjs vX.Y.Z
```

## Verify

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo xtask test             # Rust tests and helper unit tests
cargo xtask verify           # every gate, including docs and source budgets
npm test && npm run lint
```

Helper Java uses google-java-format in AOSP style; CI checks it:

```console
java -jar google-java-format-all-deps.jar --aosp --replace $(git ls-files 'device/*.java')
```

On JDK 16+ the formatter needs `--add-exports=jdk.compiler/com.sun.tools.javac.<pkg>=ALL-UNNAMED` for `api`, `code`, `file`, `parser`, `tree`, and `util`; see `.github/workflows/ci.yml`.

Live checks use one authorized device:

```console
cargo xtask benchmark-live
```

Privacy-sensitive capture checks require explicit Android permission. Do not enable them merely to satisfy an unrelated test.

## Source map

- `computer/` — engine, agent layer, adapters, protocol, artifacts, browser, and installer lifecycle. Integration tests live in `computer/tests/`.
- `device/` — authenticated Android helper and deterministic debug example. JVM unit tests live in `device/app/src/test/`.
- `tools/` — `cargo xtask`: verification, packaging, live tests, benchmarks, and documentation and source budgets.
- `install/` — lightweight npm launcher for bundled platform releases.
- `skills/` — compact operating instructions for coding agents.
