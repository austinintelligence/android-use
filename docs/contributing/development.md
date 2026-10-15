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
cargo build --locked --release --bin au
cargo xtask android          # debug helper APK and the aubench fixture
cargo xtask package          # release archive layout in dist/ (embeds the helper)
```

## Workflows

`cargo xtask workflow-policy` (also run by `check` and `verify`, so in CI) holds `.github/workflows/` to these rules: every `uses:` is pinned to a full 40-character commit SHA; a top-level `permissions:` block exists and grants no write (jobs ask for their own); no `pull_request_target`; `push` triggers name only `main` and `v*` tags; runners name a version, never `-latest`; every `cargo build|test|run|install|clippy|check|deny` has `--locked`; `cargo install` has `--version`; nothing pipes a download into a shell; every checkout sets `persist-credentials: false`; `npm ci`, never `npm install`; no `workflow_run` trigger and no attacker-controlled text (PR titles, branch names, comment bodies) expanded in a workflow. A `pull_request` workflow may not hold contents, id-token, packages, actions or pull-requests write. The check reads the YAML as text (case-insensitively, joining `\` and trailing-`|` continuations), so treat it as a tripwire that catches mistakes, not a proof: review changes under `.github/` by hand, and give them a CODEOWNERS entry. "Locked" covers Cargo and npm only; the Gradle build still resolves dependencies without verification metadata, which is remaining work. CI runs on pushes to `main` and on pull requests. `cargo-audit` and `cargo-deny` are installed at the versions pinned in `security.yml`; bump them there deliberately.

## Release

See [releasing](release.md).

## Verify

```console
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets
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
