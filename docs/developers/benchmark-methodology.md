# Benchmark methodology

android-use benchmarks measure the shipped release path on a real enrolled Android device. They are not synthetic CLI-only timing loops and they do not claim superiority over another tool unless the same public workload, hardware, versions, warmup, and measurement method are reproduced for both.

## Release-path requirements

- Use a release `au` binary and a signed, non-debuggable helper APK.
- Validate the helper package, certificate continuity, protocol, and foreground-service declaration before the run.
- Bind every command to one exact enrolled device identity; redact the serial in public reports.
- Keep one persistent `au serve --jsonl` process warm for long-running measurements.
- Record the exact code revision, release/helper versions, Android version/API, transport class, browser package/version, and benchmark fixture digest.
- Keep raw private logs outside the repository. Publish only redacted aggregate results and sanitized failure examples.

## Latency

Report p50, p95, and p99 wall latency separately for:

- cold CLI startup/help
- warm daemon status/capabilities
- persistent JSONL no-op
- semantic snapshot/choices/query
- one fused helper frame with 1, 8, 20, and 32 dependent actions
- browser navigation/download completion
- helper/daemon/browser recovery

Warm up each lane before sampling. Use at least 100 samples for short operations and include timeouts/failures in the denominator. Do not silently drop outliers.

## Token and observation density

Measure UTF-8 bytes and model-token estimates for the exact agent-visible payload, not internal binary frames. Report:

- mean/p50/p95 observation bytes
- nodes represented per kilobyte
- action receipt bytes per committed step
- bytes for dense tuples versus the repository's fixed object-shaped and legacy-rich fixtures
- omitted-field/default rules and any information intentionally excluded

Density is acceptable only if the payload remains deterministic, schema-identified, and sufficient to choose and verify the next action. A smaller payload that hides partial commits, identity changes, stale references, or errors is a regression.

## Endurance and realism

The long benchmark should run for at least 60 minutes when the environment permits and combine:

- native app semantic UI flows
- real Android browser UI, tabs, forms, back/forward, scrolling, and HTTPS downloads
- install/update/launch/force-stop/uninstall of an AU-owned benchmark APK
- at least three UI categories (system UI, browser/WebView, conventional app)
- screen rotation, screen off/on, stale-node recovery, helper restart, daemon restart, browser restart, and bounded network interruption
- mixed reads and mutations in dependency-aware batches

Track total contract calls, helper frames, semantic actions, mutations, verified commits, stale recoveries, retries, failures, browser/app restarts, downloads, and package lifecycle operations.

The current physical-device endurance evidence includes a failed-then-fixed
pair: the first 60-minute run completed workload and cleanup but exposed a
one-hour helper-session expiry as one late `E_AUTH`; after bounded host-side
reauthentication, the second 60-minute run completed 1,425 cycles with zero
errors and passed every acceptance and cleanup assertion. Keep both records in
the private artifact history when comparing future changes. A future helper
authentication, transport, or retry change must repeat the full duration
instead of relying only on a short smoke test.

## Resource stability

Capture warm, peak, and final host RSS, handles/file descriptors, and thread counts where the OS exposes them. Also capture device battery and thermal state before and after. Fail the stability gate if resource growth remains monotonic after warmup, if stale transports accumulate, or if an ambiguous commit is reported as success.

## Cleanup

The benchmark owns only artifacts named in its run manifest. On exit, including failure:

- remove benchmark-only APKs and instrumentation packages
- delete only exact downloads created by the run
- remove temporary ADB forwards and partial host artifacts
- restore the release helper after instrumentation
- restore only accessibility entries changed by android-use, preserving unrelated entries
- compare package, download, forward, helper, and accessibility state with preflight

Public reports must disclose cleanup failures and any device/API/UI surface that was not tested.

## Unfamiliar real-task evaluation

Fixture endurance is necessary for regression detection, but it is not a
measure of whether an agent can solve an unfamiliar Android task. A separate
real-task run uses one singular goal selected without exposing a precomputed
selector path or fixture-specific script. The agent must discover the UI,
choose its own bounded plan, recover from ordinary page and window changes,
and produce a verifiable outcome or an honest blocked result.

Each task records:

- the plain-language goal and the exact mutation boundary;
- the starting device/app/browser state and any user-owned state deliberately
  excluded from scope;
- agent-visible observations, action receipts, retries, stale recoveries,
  screenshots or downloads only when needed, and elapsed time;
- the authoritative outcome evidence, not a self-reported success sentence;
- cleanup and restoration proof for every resource created by the task.

An AU command receipt is never sufficient by itself. `opened`, `clicked`,
`committed`, or an HTTP/CDP response proves dispatch or transport, not the
user's goal. The task score is `success` only when its declared postcondition
is observed (for example, a page reached the requested URL, a timer is running
with the requested duration, or media is demonstrably playing). An unavailable
player, an unverified transition, or an unclosed task-created tab is recorded as
`failed` or `blocked`, even if the AU calls were fast and returned `ok`.

The first real-task suite covers three different surfaces: open-ended browser
research with live public pages, read-only Settings/Files diagnosis, and a
guarded app-store install/use/uninstall task. No task may purchase, send a
message, sign in, change an existing app or file, grant a privacy-sensitive
permission, or remove a pre-existing resource. A store task must prove that a
candidate was absent before installation and absent after exact uninstall.
These tasks complement the fixture benchmark; they do not replace its
repeatable latency and cleanup gates.

## Fresh held-out task evidence

The current tablet run used three new `gpt-5.6-luna` agents at low, medium,
and max reasoning with no fixture script or Clash-specific instructions. All
three produced an authoritative user-level result, while exposing different
recovery paths:

| Level | Singular task | Verified outcome | Friction generalized |
| --- | --- | --- | --- |
| low | Find and leave an official public park-information page | Yellowstone NPS page selected and page text verified | Android foreground snapshot lagged behind CDP; tab ownership/text state was the shorter recovery path |
| medium | Research a public NASA page and save it locally | NASA Perseids page downloaded and verified in Chrome Downloads | Files accessibility window and a malformed negative handle required one bounded visual fallback |
| max | Verify a current NASA fact and clean temporary browsing state | “Peak night: Aug. 12-13” verified; temporary tabs closed and original tabs retained | Target drift/overflow and Downloads residue required exact owned-tab cleanup |

These results are evidence of task completion, not a claim that low reasoning
is universally faster or more accurate than a person. The shared architectural
shortcuts are owned browser-target identification, compact text proof, current
foreground/tab identity, valid-handle diagnostics, and atomic postcondition plus
cleanup helpers. The separate lo-fi run remains a recorded failure because the
page opened but playback was never proven.
