# Testing

Four layers, fastest first. `cargo xtask verify` runs the first two and builds the release helper; layers 3 and 4 are run by hand, and layer 4 needs a device or emulator.

## 1. Unit and flow tests

`cargo test --locked --workspace` runs the Rust unit tests and the agent flow tests, which drive the real agent layer against a scripted fake helper that speaks the wire protocol. To write one, build `FakeLink::new(|request| Some(reply))` (`computer/src/link.rs`) from a closure that answers each helper request array (return `None` to drop the connection), and pass it to `Engine::with_link`; see `computer/src/engine.rs` for reconnect tests and `agent_tests.rs` for a full screen script. Recorded screens live in `computer/tests/fixtures/screens/`; `cargo xtask golden-capture NAME --reviewed` records another from the connected device; check the screen shows nothing personal first, because fixtures are committed (screens with an email address, phone number, or card number are refused). JVM tests for the helper are under `device/app/src/test/` (`cargo xtask test` runs both). `npm test` covers the installer, launcher, publisher, and formula renderer. The `package-smoke` CI job (Linux, macOS, Windows) builds `au`, packs the real `android-use` and `@androiduse/<platform>` packages, installs them into a fresh prefix, runs `read help` and an MCP `initialize` through `serve --mcp`, then checks the launcher exits 1 for a tampered binary and a missing manifest; run it locally with `sh install/test/package-smoke.sh target/debug/au`. It covers linux-x64, darwin-arm64 and win32-x64 with a debug `au` (no embedded helper), not the release archives.

## 2. Gates

`cargo xtask verify` also checks formatting and clippy, the workflow policy (`cargo xtask workflow-policy`), that the generated protocol reference is current (`cargo xtask docs --write` regenerates it), documentation token budgets and examples, the fixed-token and screen-token limits, artifact sizes, source budgets, that product code never names `uiautomator`, and that every version and platform list agrees (`cargo xtask version`). [Measurements](../design/measurements.md) lists the numbers.

## 3. Helper build

`cargo xtask android` builds the debug helper and, with it, the `aubench` fixture app (`device/example`; `verify` does not build the fixture): a deterministic screen with a dialog, a form, duplicate labels, a near-miss label, slow content, an icon-only button, and a long list.

## 4. Live checks

Both run against one device or emulator that has the helper installed and enabled. Live tests use the fixture app, finite capture times, and temporary state; do not point them at a personal account.

**`cargo xtask benchmark-live`** drives `au serve --mcp` over thirteen tasks and reports calls, tokens, time, and success. Variables: `AU_BIN` (default `target/dist/au`), `BENCH_RUNS` (default 3), `BENCH_OUT`, `BENCH_BASELINE` with `BENCH_GATE=1` (fail on a lower success rate or a p95 more than 20% slower), `BENCH_TRACE`, `BENCH_VERBOSE`. `BENCH_QUIET_MS=200,150,120` runs the whole benchmark once per quiet window (the host reads it as `AU_QUIET_MS`; the default is 200), writes `benchmark-live.<ms>ms.json` for each, and prints `quiet_ms=… tap_p50=… tap_p95=… success=…` so the values compare on tap latency and success.

**`cargo xtask bench-model`** measures a real agent CLI, not a script, and compares two `au` builds on one device. It runs the goals in `tools/bench/tasks.json` (fifteen: mostly Settings, plus Clock, Calendar, and Chrome). Each task has a goal, reset commands (`read ...`, `act ...`, or `adb ...` such as `am force-stop`, run before every run), and verifiers: `au` reads whose result must contain (and optionally not contain) given text, so success is decided from device state, never from the agent's claim. A run is invalid, and the agent is not started, when a reset fails or the verifiers already pass after the reset; invalid runs are counted (`invalid_runs`) and left out of the statistics. Resets and verifiers need `adb` (`AU_ADB`). Build A runs the resets and verifiers for both builds.

```bash
cargo xtask bench-model --a PATH_A --b PATH_B --cmd 'claude -p {prompt} --mcp-config {mcp} --output-format json --model {model}' --model MODEL --runs 5
```

- `--cmd` (`BENCH_CMD`) is the agent command. It is split on spaces, then `{prompt}` (the goal, kept as one argument), `{mcp}` (an MCP config file naming the build under test), and `{model}` are filled in. For Codex, use its `exec` form with a config that points at `{mcp}`. The agent's stdout goes to `target/bench-model/`.
- Runs are interleaved: every round runs every task on both builds, flipping which goes first. `--runs` (`BENCH_RUNS`, default 5) per task; `--task NAME` runs one; `--tasks FILE` uses another task file; `--timeout SECONDS` (default 300) kills a stuck agent.
- Spend guard: `--max-runs N` (default 200) and `--max-cost USD` (default 25) stop the run when reached; an unparsable number is an error. Cost is the `total_cost_usd` the agent CLI reports (a killed run is charged the costliest run so far), so without one only `--max-runs` bounds spend.
- Calls are counted by a shim (`cargo xtask bench-shim`) that the agent starts as its MCP server; it relays to `au serve --mcp` and logs each `tools/call`. Input tokens come from the last usage line the agent prints: `input_tokens` plus the cache read and creation counts (Claude shape), or `input_tokens` alone when `cached_input_tokens` is present (Codex shape); empty if it prints none.
- Output: a table, and `target/bench-model.json` (`--out`) with, per build and per task, success rate, calls, input tokens, and wall seconds as mean with a 95% confidence interval (Student t), plus A minus B with a Welch interval. Success is a 0/1 mean.

**Taking the 1.2.0 baseline.** Check out the `v1.2.0` tag in a separate worktree, run `cargo build --profile dist -p au` there, and keep that binary as A; build the current tree's binary as B. Start the reference emulator (API 36 Pixel 7, see [measurements](../design/measurements.md)) with the helper enabled, then run the command above with the same `--model` for both. Keep the JSON as the baseline for later gates; rerun A beside every B, since the model and emulator drift.

**`cargo xtask emulator-e2e`** exercises the emulator workflow on one managed emulator: create, start, setup, read and act, a refused second serve, restart with enrollment intact, and a snapshot save and restore. The first run waits while you turn on Android-Use under Settings → Accessibility on the emulator (`E2E_WINDOW=1` shows it); later runs reuse that. It also puts the emulator to sleep, locks it, disables the helper, and presses Stop in turn, and checks that `act` returns `screen_off`, `locked`, `helper_killed`, and `stopped`; it uses `adb` from `PATH` (or `AU_ADB`) for that. Variables: `E2E_NAME` (default `au-e2e`), `E2E_WINDOW=1`, `E2E_DESTROY=1`. State is under `target/e2e-home`.

## Recording the README demo

With the e2e emulator running and enabled, `python3 docs/design/baseline/mcp_demo.py target/dist/au au-e2e` runs a short Settings session over MCP, prints each result with its time, and saves the last screenshot as `capture.png`. Use an emulator so no personal data appears.

## Privacy

Camera, microphone, location, and screen-recording checks need explicit Android permission. Do not enable them only to satisfy an unrelated test.
