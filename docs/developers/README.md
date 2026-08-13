# Android Use for developers

Android Use has three main parts:

- `crates/android-use`: the native Rust host and `au` command;
- `android/aubridge`: the networkless Android helper;
- `packages/installer`: the verified Node.js bootstrapper and packaged agent skill.

The canonical agent skill is [`skills/android-use`](../../skills/android-use/SKILL.md). Its detailed references live inside that directory. `scripts/sync-public-skill.ps1` copies the canonical skill into the npm package and fails CI if the packaged copy is stale.

## First checkout

Install Rust, Node.js 20.11 or newer, JDK 17, Android SDK API 36, and Android Build Tools 36.0.0. Then run:

```sh
cargo test --workspace --all-targets
npm install
npm test
npm run docs:check
npm run skill:check
```

Build the native host with `cargo build --workspace --release`. On Windows, build the Android helper with:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-helper.ps1 -Release
```

## Where to read next

- [Architecture](architecture.md) explains process, transport, helper, browser, identity, and state boundaries.
- [Contributing](../../CONTRIBUTING.md) covers change design, tests, privacy, and pull requests.
- [Benchmark methodology](benchmark-methodology.md) separates repeatable performance evidence from real-task evaluation.
- [Release process](release.md) and [supply chain](supply-chain.md) cover packaging and publication.
- [Brand system](brand.md) contains the public naming and visual assets.

Do not commit device state, serials, tokens, recordings, screenshots, signing keys, build output, installer state, or local benchmark artifacts.
