# android-use

Fast, bounded Android control for Windows agents.

`android-use` provides the two-character `au` CLI: a Rust-first ADB client and per-user daemon with persistent shell sessions, exact hardware-identity selection, compact machine output, batch/tape execution, semantic accessibility control, CDP web control, bounded media capture, files, apps, notifications, and mock location.

The repository also contains the optional Android helper `dev.codex.aubridge`. The helper is required for semantic UI, notifications, camera, microphone, and location features. Basic coordinate control and read-only ADB commands remain usable without it.

## Install

From the standard Codex skill registry:

```powershell
npx skills add drperky20/android-use --skill android-use -g -a codex -y
```

The canonical skill source is [`skills/android-use`](skills/android-use/SKILL.md).

After the npm package is published by the release owner on Windows x64:

```powershell
npx --yes android-use@latest install --agent codex
```

The `1.0.0` GitHub prerelease is already available for source/release-asset
verification; npm publication is intentionally owner-managed and is not
claimed until the package is visible in the registry.

The installer verifies the signed release manifest, SHA-256, byte count, and staged replacement before activating `au.exe`. It never downloads an unpinned binary. `--with-helper` also downloads the helper APK; `--install-helper` installs it on the currently enrolled device.

The release installer stores host state under `%LOCALAPPDATA%\Codex\android-use` and the skill under `%USERPROFILE%\.codex\skills\android-use` (or `%CODEX_HOME%\skills\android-use`).

## First device

```powershell
au d
au u SERIAL_OR_ENDPOINT
au st
au ui snap --compact --frontier
```

Enrollment records the endpoint's reported `ro.serialno`. USB is preferred; Wi-Fi and mDNS are failover candidates only when they report the same exact hardware identity. No device serial is embedded in the public source.

## Agent-first usage

```powershell
# One persistent shell transaction, with a short settle gap between mutations.
au --delay 200 b "home; t 50% 50%; tx 'hello'; k ENTER"

# Text-first semantic control.
au ui snap --compact --frontier
au ui find "text~Allow,clickable=true#0"
au ui tap NODE_HANDLE

# Compact JSONL foreground mode.
au -w pipe --jsonl

# Raw escape hatches are intentionally broad and are not a safety boundary.
au adb -- shell getprop ro.build.version.release
au sh -- getprop ro.serialno
```

Normal success output is one line (`ok`, `ok N`, or `ok PATH`). Use `-w` for the versioned minified wire envelope, `-j` for stable JSON, and `--out PATH` for large or binary results. Screenshots, camera, microphone, and recordings never enter a normal terminal transcript as raw bytes.

## Architecture

```text
Codex skill -> au CLI -> current-user named-pipe daemon -> persistent ADB transport
                                      |\
                                      | +-- CDP forward for Chrome
                                      | +-- authenticated ADB-forwarded AU Bridge
                                      |     (Accessibility, media, notification, location)
                                      +-- bounded child-process and artifact manager
```

The helper exposes no network listener and requests no `INTERNET` permission. It uses an authenticated abstract local socket reachable only through an AU-owned ADB forward. The Windows daemon uses a versioned, length-prefixed named-pipe protocol restricted to the current user.

## Build from source

Requirements: Windows x64, Rust 1.94.0, JDK 17, Android SDK API 36, Build Tools 36.0.0, Gradle 9.1.0, and Node.js 20.11+ for the installer package.

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo build --workspace --release
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-helper.ps1 -Release
npm test --workspace packages/installer
```

The helper build creates or reuses a machine-local signing key outside the repository. Private signing material, device state, recordings, benchmark artifacts, and local configuration are excluded from the public repository.

## Scope and limitations

- Host support is currently Windows x64.
- Root access is not required.
- Android semantic, notification, media, and mock-location features require one-time user-granted helper capabilities.
- Android firmware, OEM permissions, Chrome availability, camera hardware, and audio routing can make individual capabilities unsupported; `au doctor` reports capability errors explicitly.
- Wi-Fi performance depends on the endpoint. Failover is identity-safe even when latency is not.
- Raw ADB and shell are available for advanced users, but authorization and confirmation remain an agent/skill policy concern rather than a bypassable string denylist.

See [`docs/installation.md`](docs/installation.md), [`docs/architecture.md`](docs/architecture.md), [`docs/supply-chain.md`](docs/supply-chain.md), and [`references/command-map.md`](references/command-map.md).

## Upstream references

- [Open Agent Skills CLI](https://github.com/vercel-labs/skills) for the `npx skills add` installation flow.
- [Android AccessibilityService API](https://developer.android.com/reference/android/accessibilityservice/AccessibilityService) for semantic UI control.
- [Official scrcpy releases](https://github.com/Genymobile/scrcpy/releases) for the pinned v4.1 integration.
- [npm trusted publishing](https://docs.npmjs.com/trusted-publishers/) for owner-managed provenance publication.

## Contributing and security

Read [`CONTRIBUTING.md`](CONTRIBUTING.md) before opening a change. Report security issues privately according to [`SECURITY.md`](SECURITY.md). Do not include device serials, tokens, recordings, screenshots, signing keys, or private benchmark traces in issues or pull requests.

## License

MIT. See [`LICENSE`](LICENSE).
