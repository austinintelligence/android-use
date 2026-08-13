# Competitive landscape

This is a source comparison, not a performance leaderboard. It was refreshed
against each project's official repository or documentation on 2026-08-12.
Latency, token, success-rate, and resource claims are withheld until the same
fixed workload is reproduced on the same host and device.

| System | Primary shape | Android-side component | Semantic UI | Browser/WebView | Media | Deterministic batching |
| --- | --- | --- | --- | --- | --- | --- |
| android-use | Native Rust CLI/daemon plus JSONL/MCP contract | Non-debuggable, authenticated Accessibility helper with no `INTERNET` permission | Dense choices, selectors, generations, deltas, device-plan receipts | Direct Chrome CDP with owned forwards | Bounded artifacts; optional scrcpy | Dependency-bound `plan.run`, proof fast path, shell batch, recipes |
| [Mobile MCP](https://github.com/mobile-next/mobile-mcp) / [mobilecli](https://github.com/mobile-next/mobilecli) | Node MCP server over mobilecli | mobilecli can install an agent; Android can use ADB for many operations | Accessibility-tree agent tools | mobilecli documents WebView inspection and DOM evaluation | Screen capture/stream tooling | MCP tool calls; no equivalent commit-receipt claim was found in the audited public overview |
| [Mobilewright](https://github.com/mobile-next/mobilewright) | Node testing API over mobilecli | mobilecli agent where required | Playwright-style role/label locators and auto-wait | Mobile/webview support inherited from its device layer | Device-layer capture | Repeatable test code and assertions |
| [Maestro](https://github.com/mobile-dev-inc/Maestro) | Declarative YAML/JVM test runner | Host APK plus an instrumentation/server APK on Android | Rendered-UI selectors and assertions | Mobile web flows are supported by Maestro's test surface | Screenshots/recording in test tooling | Declarative flow commands, conditions, loops, and subflows |
| [Appium UiAutomator2](https://github.com/appium/appium-uiautomator2-driver) | Appium Node server plus WebDriver driver | UiAutomator2 server/test packages | WebDriver/UIAutomator locators and gestures | Native/web contexts through a matching Chromedriver | Screenshot and MJPEG facilities | Client-language test programs and W3C actions |
| [scrcpy](https://github.com/Genymobile/scrcpy) | Native low-latency visual mirror/control | Ephemeral server; no app left installed | Not an accessibility-tree agent protocol | Visual/input control rather than DOM automation | Best-in-class mirroring, recording, audio, camera, HID | Input/control stream rather than semantic transaction receipts |

## Different product goals

Mobilewright, Maestro, and Appium primarily optimize authored test suites.
Mobile MCP optimizes broad MCP-accessible exploration across iOS and Android.
scrcpy optimizes high-quality visual mirroring and input. android-use is
deliberately Android-focused and optimizes the model-facing loop: exact device
identity, small observations, bounded multi-step mutation receipts, local-only
helper authentication, and explicit unknown-commit recovery.

Those differences make raw feature-count comparisons misleading. In
particular, scrcpy remains the preferred optional visual-stream dependency
rather than code android-use should fork, while Appium/Maestro remain stronger
choices when an organization already owns large WebDriver or declarative test
suites.

## Reproducible comparison requirements

An apples-to-apples run must publish:

- exact tool/package revisions and install commands;
- host, platform-tools, device/API, transport, and app fixture digest;
- warmup and at least p50/p95/p99 sample distributions;
- exact agent-visible request/response bytes and tokenizer selection;
- package/device residue before and after;
- failures and timeouts in the denominator;
- the same native form/dialog/scroll task and browser form/download task.

Until those lanes are complete, public copy may say what android-use measured
about itself, but not that it is universally faster, smaller, or more capable
than these projects.
