# Testing

Four layers, fastest first. `cargo xtask verify` runs the first three; the last needs a device or emulator.

## 1. Unit and flow tests

`cargo test --workspace` runs the Rust unit tests and the agent flow tests, which drive the real agent layer against a scripted fake helper that speaks the wire protocol. Recorded screens live in `computer/tests/fixtures/screens/`; `cargo xtask golden-capture NAME` records another from the connected device. JVM tests for the helper are under `device/app/src/test/` (`cargo xtask test` runs both). `npm test` covers the installer, launcher, publisher, and formula renderer.

## 2. Gates

`cargo xtask verify` also checks formatting and clippy, that the generated protocol reference is current (`cargo xtask docs --write` regenerates it), documentation token budgets and examples, the fixed-token and screen-token limits, artifact sizes, source budgets, that product code never names `uiautomator`, and that every version and platform list agrees (`cargo xtask version`). [Measurements](../design/measurements.md) lists the numbers.

## 3. Helper build

`cargo xtask android` builds the debug helper and the `aubench` fixture app (`device/example`): a deterministic screen with a dialog, a form, duplicate labels, a near-miss label, slow content, an icon-only button, and a long list.

## 4. Live checks

Both run against one device or emulator that has the helper installed and enabled. Live tests use the fixture app, finite capture times, and temporary state; do not point them at a personal account.

**`cargo xtask benchmark-live`** drives `au serve --mcp` over thirteen tasks and reports calls, tokens, time, and success. Variables: `AU_BIN` (default `target/dist/au`), `BENCH_RUNS` (default 3), `BENCH_OUT`, `BENCH_BASELINE` with `BENCH_GATE=1` (fail on a lower success rate or a p95 more than 20% slower), `BENCH_TRACE`, `BENCH_VERBOSE`.

**`cargo xtask emulator-e2e`** exercises the emulator workflow on one managed emulator: create, start, setup, read and act, a refused second serve, restart with enrollment intact, and a snapshot save and restore. The first run waits while you turn on Android-Use under Settings → Accessibility on the emulator (`E2E_WINDOW=1` shows it); later runs reuse that. Variables: `E2E_NAME` (default `au-e2e`), `E2E_WINDOW=1`, `E2E_DESTROY=1`. State is under `target/e2e-home`.

## Recording the README demo

With the e2e emulator running and enabled, `python3 docs/design/baseline/mcp_demo.py target/dist/au au-e2e` runs a short Settings session over MCP, prints each result with its time, and saves the last screenshot as `capture.png`. Use an emulator so no personal data appears.

## Privacy

Camera, microphone, location, and screen-recording checks need explicit Android permission. Do not enable them only to satisfy an unrelated test.
