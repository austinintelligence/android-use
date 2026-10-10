# Development

The host is Rust; the helper is a small Android Java app. Read [CONTRIBUTING.md](../../CONTRIBUTING.md) and [SECURITY.md](../../SECURITY.md) before changing trust boundaries, and [architecture](../concepts/architecture.md) for the module map.

## Toolchain

| Tool | Version | Used for |
| --- | --- | --- |
| Rust | 1.94 (`rust-toolchain.toml`) | Host binary, tests, `cargo xtask` |
| Node.js | 22.18+ | Installer tests and lint |
| JDK | 17+ (21 for the Java formatter) | Helper build |
| Gradle | 9.1.0 | Helper build (`AU_GRADLE` may point to it) |
| Android SDK | `platforms;android-36`, `build-tools;36.0.0`, `platform-tools` | Helper build, `adb` |
| Android emulator | `emulator` and a `google_apis` system image (`au emulator install`) | Emulator targets and live checks |

Point `ANDROID_HOME` (or `ANDROID_SDK_ROOT`) at the SDK and `JAVA_HOME` at the JDK.

## Build

```console
cargo build --release --bin au
cargo xtask android          # debug helper APK and the aubench fixture
cargo xtask package          # release archive layout in dist/ (embeds the helper)
```

## Release

See [releasing](release.md).

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

Live checks (a benchmark and an emulator end-to-end run) are in [testing](testing.md).

## Source map

- `computer/` — engine, agent layer, adapters, protocol, artifacts, browser, and installer lifecycle. Integration tests live in `computer/tests/`.
- `device/` — authenticated Android helper and deterministic debug example. JVM unit tests live in `device/app/src/test/`.
- `tools/` — `cargo xtask`: verification, packaging, live tests, benchmarks, and documentation and source budgets.
- `install/` — the npm launcher and its publisher, install scripts, Homebrew formula renderer, and `platforms.json`, the one list of release platforms.
- `skills/` — compact operating instructions for coding agents.
