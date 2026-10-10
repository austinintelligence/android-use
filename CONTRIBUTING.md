# Contributing to Android Use

Thanks for helping. Start with [development](docs/contributing/development.md) for the toolchain and source layout, and [testing](docs/contributing/testing.md) for the checks.

## Before changing code

- Keep the change focused and preserve unrelated work.
- Do not add device-specific defaults, serials, tokens, private URLs, screenshots, recordings, or personal app data.
- Keep local screenshots, recordings, and run evidence outside the checkout (or under an ignored temporary directory); never leave them in the repository root.
- Prefer semantic actions and persistent sessions over repeated screenshots or process launches.
- Give every operation a deadline, output bound, cancellation path, and clear failure.
- Require explicit output paths for binary or large data.

New commands need tests for valid use, malformed input, timeouts, partial completion, and cleanup. Agent-facing command forms live once, in `computer/src/spec.rs`; run `cargo xtask docs --write` after changing them. If an operation changes device state, define the authoritative postcondition before implementation.

## Code style

Rust is formatted with `rustfmt` and must pass clippy with `-D warnings`. Helper Java is formatted with google-java-format in AOSP style (`--aosp`); CI rejects unformatted Java. Prefer small, named functions over dense one-line methods: the helper is a security boundary and should be easy to review. Source budgets in `cargo xtask size` measure formatted code.

## Run the checks

From the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
npm test
npm run lint
cargo xtask verify
```

If you changed the Android helper, build it with `cargo xtask android` or `cargo xtask package` and run the real-device benchmark with `cargo xtask benchmark-live`. Live-device tests must use a harmless fixture or test activity, finite capture durations, temporary artifact storage, and independent cleanup proof. Do not manipulate a personal app merely to demonstrate a feature.

## Open a pull request

Explain:

- what changes for users or agents;
- any compatibility or security impact;
- the checks you ran;
- Android versions, device makers, or capabilities you could not test.

Keep generated build output and local state out of the commit. A reviewer should be able to understand the contract change without reading benchmark archives or private device evidence.

## Release maintenance

`VERSION` is the public version source of truth. Do not edit package, CLI, or Android version strings independently; `cargo xtask version` checks that every shipped surface agrees. Releases follow [releasing](docs/contributing/release.md): rehearse the workflow by hand, then tag.
