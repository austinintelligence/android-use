# android-use

Control Android from a Windows agent without turning every action into brittle screen coordinates.

`android-use` is a Windows-first control layer for Android phones and tablets. It gives agents one small, scriptable surface for inspecting a device, finding controls by meaning, tapping and typing, opening apps, working with Chrome, capturing bounded media, reading notifications, and more.

The project is built around a short command: `au`.

## Why it exists

Android automation often breaks when a device reconnects, a screen moves, or a command prints far too much output. `android-use` keeps the path predictable:

- exact device identity instead of trusting a friendly name or a remembered IP address;
- semantic accessibility actions when the Android helper is available;
- a warm per-user daemon so repeated actions do not pay the full connection cost;
- compact, bounded output that is easier for an agent to reason about;
- explicit limits around screenshots, recordings, shell commands, and other artifacts.

## What you can do

| Goal | Example | Extra Android helper needed? |
| --- | --- | --- |
| Inspect the current screen | `au ui snap --compact --frontier` | Usually yes |
| Send a short batch of basic actions | `au --delay 200 b "home; t 50% 50%; tx 'hello'; k ENTER"` | No |
| Find a control by text or properties | `au ui find "text~Allow,clickable=true#0"` | Yes |
| Work with Chrome through CDP | See the browser references | No helper, but Chrome is required |
| Use camera, microphone, notifications, or mock location | See the helper setup | Yes |

The optional helper is `dev.codex.aubridge`. Basic coordinate control and read-only ADB commands remain useful without it.

## Install on Windows

Install the Codex skill from the public repository:

```powershell
npx skills add austinintelligence/android-use --skill android-use -g -a codex -y
```

When a published npm release is available, the verified host installer can be used as well:

```powershell
npx --yes android-use@latest install --agent codex
```

The installer verifies the release manifest, SHA-256 digest, byte count, and staged replacement before activating `au.exe`. `--with-helper` keeps a verified helper APK in the local version store, and `--install-helper` installs it on the enrolled device.

The installer stores host state under `%LOCALAPPDATA%\Codex\android-use` and the skill under `%USERPROFILE%\.codex\skills\android-use` (or `%CODEX_HOME%\skills\android-use`).

## Connect your first device

Enable USB debugging or Wireless debugging on Android, authorize the computer, and then run:

```powershell
au d
au u SERIAL_OR_ENDPOINT
au st
au ui snap --compact --frontier
```

Enrollment records the endpoint's reported `ro.serialno`. USB is preferred; Wi-Fi and mDNS are failover candidates only when they report the same exact hardware identity.

## A simple mental model

```text
Codex or another agent
          |
          v
       au CLI  ->  Windows per-user daemon  ->  ADB  ->  Android device
                                      \
                                       +-> optional AU Bridge
                                           (accessibility, media, notifications, location)
```

The CLI is intentionally small. The daemon keeps transports warm, owns the current-user named pipe, and manages bounded child processes and artifacts. The Android helper stays local to the device and exposes no network listener of its own.

## Output made for agents

Normal success output is one line such as `ok`, `ok N`, or `ok PATH`. Use:

- `-w` for the versioned minified wire envelope;
- `-j` for stable JSON;
- `--out PATH` for large or binary results.

Screenshots, camera frames, microphone data, and recordings do not get dumped into a normal terminal transcript as raw bytes.

## Scope and limitations

- Host support is currently Windows x64.
- Root access is not required.
- Semantic UI, notification, media, and mock-location features require one-time helper capabilities on Android.
- Android firmware, OEM permissions, Chrome availability, camera hardware, and audio routing can affect individual capabilities. `au doctor` reports capability errors explicitly.
- Raw ADB and shell remain available for advanced users. They are escape hatches, not a substitute for an agent's approval and policy layer.

## Documentation

Start with [`docs/README.md`](docs/README.md), then choose the depth you need:

- [`docs/installation.md`](docs/installation.md) — first setup, helper installation, and device enrollment;
- [`docs/architecture.md`](docs/architecture.md) — runtime layers, identity, and state ownership;
- [`docs/limitations.md`](docs/limitations.md) — supported and unsupported environments;
- [`docs/security.md`](docs/security.md) — security boundaries and safe use;
- [`docs/supply-chain.md`](docs/supply-chain.md) — release verification and public-tree checks;
- [`docs/benchmarks.md`](docs/benchmarks.md) — performance methodology and results.

## Build from source

Maintainers need Windows x64, Rust 1.94.0, JDK 17, Android SDK API 36, Build Tools 36.0.0, Gradle 9.1.0, and Node.js 20.11+ for the installer package.

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo build --workspace --release
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-helper.ps1 -Release
npm test --workspace packages/installer
```

The helper build creates or reuses a machine-local signing key outside the repository. Private signing material, device state, recordings, benchmark artifacts, and local configuration do not belong in the public tree.

## Contributing and security

Read [`CONTRIBUTING.md`](CONTRIBUTING.md) before opening a change. Report security issues privately according to [`SECURITY.md`](SECURITY.md). Do not include device serials, tokens, recordings, screenshots, signing keys, or private benchmark traces in issues or pull requests.

## License

MIT. See [`LICENSE`](LICENSE).

